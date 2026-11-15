//! WebSocket server and request dispatch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use muster_core::store::{title_source, CardPatch, NewCard};
use muster_core::tmux::{SessionBackend, SessionSpec};
use muster_core::util::{err, hex, invalid, sha256, shell_quote};
use muster_core::{agents, attach, worktree, Result};
use muster_proto::*;
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::log;
use crate::state::{Conn, State};

const OUTBOUND_QUEUE: usize = 1024;

/// Parts a `project.summary` range may be cut into (a year of days).
const SUMMARY_MAX_SPLITS: usize = 366;

pub async fn serve(state: Arc<State>, listener: TcpListener) {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                log!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let _ = stream.set_nodelay(true);
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(state, stream).await {
                log!("connection: {e}");
            }
        });
    }
}

#[allow(clippy::result_large_err)]
fn check_path(req: &Request, resp: Response) -> std::result::Result<Response, ErrorResponse> {
    let fail = |status: StatusCode, msg: &str| {
        let mut r = ErrorResponse::new(Some(msg.into()));
        *r.status_mut() = status;
        Err(r)
    };
    if req.uri().path() != "/ws" {
        return fail(StatusCode::NOT_FOUND, "not found");
    }
    // Any web page in the user's browser can open a socket to 127.0.0.1, so only the
    // app's own origins (and clients that send none, like the CLI) are let in.
    if let Some(origin) = req.headers().get("origin") {
        if !origin.to_str().is_ok_and(origin_allowed) {
            return fail(StatusCode::FORBIDDEN, "origin not allowed");
        }
    }
    Ok(resp)
}

/// The app (`tauri://localhost`) and its dev server: `MUSTER_DEV_ORIGINS` (comma
/// separated, exact), default the Tauri dev port on localhost and 127.0.0.1.
pub fn origin_allowed(origin: &str) -> bool {
    if origin == "tauri://localhost" {
        return true;
    }
    let dev = std::env::var("MUSTER_DEV_ORIGINS")
        .unwrap_or_else(|_| "http://localhost:1420,http://127.0.0.1:1420".to_string());
    dev.split(',').map(str::trim).any(|o| !o.is_empty() && o == origin)
}

enum Reply {
    Value(Value),
    /// The handler already queued its response (used by `session.attach`).
    Sent,
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Reply> {
    Ok(Reply::Value(serde_json::to_value(v).expect("serialise")))
}

/// Who is on the other end of a connection.
#[derive(Clone, PartialEq)]
enum Role {
    /// Before `hello`.
    None,
    /// The master token: the app, the CLI run by the user.
    Master,
    /// A card's own session token: an agent acting for that card only. Holds the card
    /// id and the token's hash, re-checked on every call so a revoked token stops
    /// working on connections opened before the revocation.
    Card(String, String),
}

struct Ctx {
    state: Arc<State>,
    conn_id: u64,
    conn: Conn,
    role: Role,
}

async fn connection(state: Arc<State>, stream: TcpStream) -> std::result::Result<(), String> {
    let mut cfg = WebSocketConfig::default();
    cfg.max_message_size = Some(16 << 20);
    cfg.max_frame_size = Some(16 << 20);
    let ws = tokio_tungstenite::accept_hdr_async_with_config(stream, check_path, Some(cfg))
        .await
        .map_err(|e| e.to_string())?;
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Message>(OUTBOUND_QUEUE);
    let kill = Arc::new(Notify::new());
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut ctx = Ctx { conn_id: state.next_conn_id(), state, conn: Conn { tx, kill: kill.clone() }, role: Role::None };
    let period =
        Duration::from_millis(std::env::var("MUSTER_PING_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000));
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let (mut seen, mut pinged) = (false, false);

    loop {
        let msg = tokio::select! {
            m = source.next() => m,
            _ = kill.notified() => {
                log!("closing connection {}: client too slow", ctx.conn_id);
                break;
            }
            _ = heartbeat.tick() => {
                // Anything received since the last ping counts as alive.
                if !std::mem::replace(&mut seen, false) && pinged {
                    log!("closing connection {}: no answer to ping", ctx.conn_id);
                    break;
                }
                pinged = true;
                ctx.conn.send(Message::Ping(Default::default()));
                continue;
            }
        };
        let Some(Ok(msg)) = msg else { break };
        seen = true;
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let frame: RequestFrame = match serde_json::from_str(text.as_str()) {
            Ok(f) => f,
            Err(e) => {
                log!("bad frame from connection {}: {e}", ctx.conn_id);
                continue;
            }
        };
        let id = frame.id;
        if ctx.role == Role::None && frame.m != "hello" {
            send_res(&ctx.conn, ResponseFrame::err(id, ErrorBody::new(ErrorCode::Unauthorized, "hello first")));
            let _ = ctx.conn.tx.send(Message::Close(None)).await;
            break;
        }
        let result = match Method::parse(&frame.m, frame.p) {
            Ok(m) => match ctx.role.clone() {
                Role::Card(me, hash) => match restrict(&ctx.state, &me, &hash, m) {
                    Ok(Restricted::Call(m)) => handle(&mut ctx, id, m).await,
                    Ok(Restricted::Done(v)) => Ok(Reply::Value(v)),
                    Err(e) => Err(e),
                },
                _ => handle(&mut ctx, id, m).await,
            },
            Err(e) => Err(e),
        };
        match result {
            Ok(Reply::Value(v)) => send_res(&ctx.conn, ResponseFrame::ok(id, v)),
            Ok(Reply::Sent) => {}
            Err(e) => {
                // A failed hello ends the connection; a refused call by a card does not.
                let fatal = e.code == ErrorCode::Unauthorized && ctx.role == Role::None;
                send_res(&ctx.conn, ResponseFrame::err(id, e));
                if fatal {
                    let _ = ctx.conn.tx.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    }

    ctx.state.clients.remove(ctx.conn_id);
    let attached = ctx.state.hub.cards_of(ctx.conn_id);
    ctx.state.hub.detach_all(ctx.conn_id);
    ctx.state.agents.chat_unsubscribe_all(ctx.conn_id);
    ctx.state.stats.unsubscribe(ctx.conn_id);
    for id in attached {
        if ctx.state.hub.subscriber_count(&id) == 0 && ctx.state.has_session(&id) {
            ctx.state.scrollback.save(&ctx.state, &id).await;
        }
    }
    drop(ctx);
    let _ = writer.await;
    Ok(())
}

fn send_res(conn: &Conn, r: ResponseFrame) {
    conn.send(Message::text(serde_json::to_string(&r).expect("serialise")));
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn handle(ctx: &mut Ctx, req_id: u64, m: Method) -> Result<Reply> {
    let st = ctx.state.clone();
    match m {
        Method::Hello(p) => {
            // A valid token always gets in: the lockout below only slows down guessing,
            // so nobody can lock the app out by sending bad tokens.
            let role = if constant_time_eq(p.token.as_bytes(), st.token.as_bytes()) {
                Role::Master
            } else {
                let hash = hex(&sha256(p.token.as_bytes()));
                let card = st.store.lock().unwrap().card_for_token_hash(&hash)?;
                match card {
                    Some(card) if !p.token.is_empty() => Role::Card(card, hash),
                    _ => {
                        let locked = st.hello_locked();
                        st.hello_failed();
                        if locked {
                            // Slow down a guesser; other connections are unaffected.
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            return Err(err(ErrorCode::Unauthorized, "too many failed attempts; try again later"));
                        }
                        return Err(err(ErrorCode::Unauthorized, "bad token"));
                    }
                }
            };
            if p.protocol != PROTOCOL_VERSION {
                return Err(invalid(format!(
                    "unsupported protocol {}; this daemon speaks {PROTOCOL_VERSION}",
                    p.protocol
                )));
            }
            if ctx.role == Role::None {
                // Card connections get replies only, not the board's event stream.
                if role == Role::Master {
                    st.clients.add(ctx.conn_id, ctx.conn.clone());
                }
                ctx.role = role;
            }
            to_value(HelloResult { protocol: PROTOCOL_VERSION, daemon_version: env!("CARGO_PKG_VERSION").into() })
        }

        Method::ProjectList(_) => to_value(ProjectListResult { projects: st.store.lock().unwrap().list_projects()? }),

        Method::ProjectAdd(p) => {
            let path = std::fs::canonicalize(&p.path).map_err(|e| invalid(format!("{}: {e}", p.path)))?;
            if !path.is_dir() {
                return Err(invalid(format!("{} is not a directory", path.display())));
            }
            let name = p
                .name
                .filter(|n| !n.trim().is_empty())
                .or_else(|| path.file_name().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "project".into());
            let (project, columns) =
                st.store.lock().unwrap().add_project(&path.to_string_lossy(), &name, p.color.as_deref())?;
            st.clients.broadcast(&Event::ProjectChanged(ProjectChanged::Updated { project: project.clone() }));
            for column in columns {
                st.clients.broadcast(&Event::ColumnChanged(ColumnChanged::Updated { column }));
            }
            to_value(ProjectResult { project })
        }

        Method::ProjectUpdate(p) => {
            let changed = st.store.lock().unwrap().update_project(
                &p.id,
                p.name.as_deref(),
                p.color.as_deref(),
                p.position,
                p.worktree_setup.as_ref().map(|w| w.as_deref()),
            )?;
            for project in &changed {
                st.clients.broadcast(&Event::ProjectChanged(ProjectChanged::Updated { project: project.clone() }));
            }
            to_value(ProjectResult { project: changed[0].clone() })
        }

        Method::ProjectRemove(p) => {
            let mut store = st.store.lock().unwrap();
            store.get_project(&p.id)?;
            if store.list_cards(&p.id)?.iter().any(|c| st.has_session(&c.id)) {
                return Err(err(ErrorCode::Conflict, "the project has cards with live sessions"));
            }
            store.remove_project(&p.id)?;
            drop(store);
            st.clients.broadcast(&Event::ProjectChanged(ProjectChanged::Removed { removed_id: p.id }));
            to_value(Empty {})
        }

        Method::BoardGet(p) => {
            refresh_activity(&st).await;
            let store = st.store.lock().unwrap();
            let project = store.get_project(&p.project_id)?;
            let columns = store.list_columns(&p.project_id)?;
            let cards = store.list_cards(&p.project_id)?.into_iter().map(|c| st.decorate(c)).collect();
            let links = store.project_links(&p.project_id)?;
            to_value(BoardResult { project, columns, cards, links })
        }

        Method::DocsList(p) => {
            let project = st.store.lock().unwrap().get_project(&p.project_id)?;
            let (docs, truncated) = crate::docs::list(Path::new(&project.path));
            to_value(DocsListResult { docs, truncated })
        }

        Method::DocsRead(p) => {
            let project = st.store.lock().unwrap().get_project(&p.project_id)?;
            let (content, truncated, modified_at) = crate::docs::read(Path::new(&project.path), &p.path)?;
            to_value(DocsReadResult { path: p.path, content, modified_at, truncated })
        }

        Method::DocsWrite(p) => {
            let project = st.store.lock().unwrap().get_project(&p.project_id)?;
            let root = Path::new(&project.path);
            let doc = crate::docs::write(root, &p.path, &p.content, p.create, p.expected_modified_at)?;
            to_value(DocsWriteResult { doc })
        }

        Method::ProjectSummary(p) => {
            if p.to <= p.from {
                return Err(invalid("the range must end after it starts"));
            }
            if p.splits.len() > SUMMARY_MAX_SPLITS {
                return Err(invalid(format!("at most {SUMMARY_MAX_SPLITS} splits")));
            }
            let (cards, rows, texts, paragraph) = {
                let store = st.store.lock().unwrap();
                store.get_project(&p.project_id)?;
                let (cards, rows) = store.summary_data(&p.project_id, p.from, p.to)?;
                let texts = store.card_texts(&p.project_id, p.from, p.to)?;
                (cards, rows, texts, store.project_text(&p.project_id, p.from, p.to)?)
            };
            let now = muster_core::util::now_ms();
            let mut r = muster_core::summary::build(&cards, &rows, p.from, p.to, &p.splits, now);
            muster_core::summary::attach_texts(&mut r, &texts, paragraph.as_ref());
            r.summarizing = crate::summarize::progress(&p.project_id, p.from, p.to);
            for c in r.cards.iter_mut().chain(r.days.iter_mut().flat_map(|d| d.cards.iter_mut())) {
                c.has_session = st.has_session(&c.id);
            }
            for c in r.active.iter_mut() {
                c.has_session = st.has_session(&c.id);
            }
            to_value(r)
        }

        Method::ProjectSummarize(p) => {
            // Tests point this at a stand-in; otherwise the `claude` the user's shell finds.
            let claude = match std::env::var_os("MUSTER_CLAUDE_BIN").filter(|p| !p.is_empty()) {
                Some(p) => Some(PathBuf::from(p)),
                None => {
                    let shell = st.backend.shell().to_string();
                    st.installed_agents.get_or_init(|| find_installed(shell)).await.get("claude").cloned()
                }
            };
            to_value(crate::summarize::start(&st, &p, claude)?)
        }

        Method::ColumnCreate(p) => {
            let column =
                st.store.lock().unwrap().create_column(&p.project_id, &p.title, p.color.as_deref(), p.category)?;
            st.clients.broadcast(&Event::ColumnChanged(ColumnChanged::Updated { column: column.clone() }));
            to_value(ColumnResult { column })
        }

        Method::ColumnUpdate(p) => {
            let changed = st.store.lock().unwrap().update_column(
                &p.id,
                p.title.as_deref(),
                p.color.as_deref(),
                p.category,
                p.position,
            )?;
            for column in &changed {
                st.clients.broadcast(&Event::ColumnChanged(ColumnChanged::Updated { column: column.clone() }));
            }
            to_value(ColumnResult { column: changed[0].clone() })
        }

        Method::ColumnDelete(p) => {
            let (col, moved, shifted) = st.store.lock().unwrap().delete_column(&p.id, &p.move_cards_to)?;
            for card in moved {
                st.emit_card(card);
            }
            for column in shifted {
                st.clients.broadcast(&Event::ColumnChanged(ColumnChanged::Updated { column }));
            }
            st.clients.broadcast(&Event::ColumnChanged(ColumnChanged::Removed {
                removed_id: col.id,
                project_id: col.project_id,
            }));
            to_value(Empty {})
        }

        Method::CardCreate(p) => {
            let agent = agents::find(&p.agent_id)
                .ok_or_else(|| invalid(format!("unknown or unsupported agent {:?}", p.agent_id)))?;
            let project = st.store.lock().unwrap().get_project(&p.project_id)?;
            // Validate the column before doing anything with side effects.
            let col = st.store.lock().unwrap().get_column(&p.column_id)?;
            if col.project_id != project.id {
                return Err(invalid("column is not in that project"));
            }
            let resume = p.resume_session_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
            if let Some(sid) = resume {
                if sid.len() > 128 || !sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(invalid("the session id may only contain letters, digits, - and _"));
                }
                if p.agent_id == "shell" {
                    return Err(invalid("a terminal card has no session to resume"));
                }
                if p.worktree.is_some() {
                    return Err(invalid("a resumed session runs in its own folder, not a new worktree"));
                }
            }
            let (cwd, branch) = match &p.worktree {
                Some(w) => {
                    let path = worktree::create(Path::new(&project.path), &w.branch).await?;
                    (path.to_string_lossy().into_owned(), Some(w.branch.clone()))
                }
                None => {
                    let cwd = p.cwd.clone().unwrap_or_else(|| project.path.clone());
                    if !Path::new(&cwd).is_dir() {
                        return Err(invalid(format!("cwd {cwd} is not a directory")));
                    }
                    (cwd, None)
                }
            };
            let (title, source) = match p.title.clone().filter(|t| !t.trim().is_empty()) {
                Some(t) => (t, title_source::USER),
                None => match p.prompt.as_deref().and_then(crate::agents::title_from_prompt) {
                    Some(t) => (t, title_source::PROMPT),
                    None => (placeholder_title(&p.agent_id, agent.label, &cwd), title_source::PLACEHOLDER),
                },
            };
            let card = st.store.lock().unwrap().create_card(NewCard {
                project_id: &p.project_id,
                column_id: &p.column_id,
                title: &title,
                agent_id: &p.agent_id,
                cwd: &cwd,
                branch: branch.as_deref(),
                prompt: p.prompt.as_deref(),
                agent_session_id: resume,
                title_source: source,
            })?;
            let card = if p.start {
                match start_card(&st, card.clone()).await {
                    Ok(c) => c,
                    Err(e) => {
                        // Keep the draft so the user can fix and retry.
                        st.emit_card(card);
                        return Err(e);
                    }
                }
            } else {
                card
            };
            let card = st.decorate(card);
            st.emit_card(card.clone());
            to_value(CardResult { card })
        }

        Method::CardUpdate(p) => {
            let editor = crate::context::author(&st, p.from_card_id.as_deref())?;
            let card = st.store.lock().unwrap().update_card(
                &p.id,
                CardPatch {
                    title: p.title,
                    prompt: p.prompt,
                    notes: p.notes,
                    priority: p.priority,
                    due_at: p.due_at,
                    title_auto: p.title_auto,
                    keep_running: p.keep_running,
                    editor: Some(editor),
                },
            )?;
            let card = st.decorate(card);
            st.emit_card(card.clone());
            to_value(CardResult { card })
        }

        Method::CardMove(p) => {
            let card =
                st.store.lock().unwrap().move_card(&p.id, &p.column_id, p.before.as_deref(), p.after.as_deref())?;
            let card = st.decorate(card);
            st.emit_card(card.clone());
            to_value(CardResult { card })
        }

        Method::CardDelete(p) => {
            let card = st.store.lock().unwrap().get_card(&p.id)?;
            if st.has_session(&card.id) {
                if !p.kill_session {
                    return Err(err(ErrorCode::Conflict, "the card has a live session; pass killSession: true"));
                }
                st.hub.drop_session(&card.id);
                st.mark_killed(&card.id);
                if let Err(e) = st.backend.kill(&card.id).await {
                    if st.backend.exists(&card.id).await {
                        return Err(e);
                    }
                }
            }
            st.store.lock().unwrap().delete_card(&card.id)?;
            remove_card_files(&st, &card.id);
            st.scrollback.forget(&card.id);
            let _ = st.store.lock().unwrap().prune_activity();
            st.reconcile().await;
            st.clients.broadcast(&Event::CardChanged(CardChanged::Removed {
                removed_id: card.id,
                project_id: card.project_id,
            }));
            to_value(Empty {})
        }

        Method::CardStart(p) => {
            let card = st.store.lock().unwrap().get_card(&p.id)?;
            let setup_failed = card.setup.as_ref().is_some_and(|s| s.state == SetupState::Failed);
            if setup_failed && st.has_session(&card.id) {
                // Start over: a new session reruns the setup.
                st.hub.drop_session(&card.id);
                st.mark_killed(&card.id);
                let _ = st.backend.kill(&card.id).await;
                st.reconcile().await;
            }
            let card = st.store.lock().unwrap().get_card(&p.id)?;
            let card = if st.has_session(&card.id) && st.backend.exists(&card.id).await {
                card
            } else {
                start_card(&st, card).await?
            };
            let card = st.decorate(card);
            st.emit_card(card.clone());
            to_value(CardResult { card })
        }

        Method::CardMarkRead(p) => {
            let card = st.store.lock().unwrap().get_card(&p.id)?;
            st.agents.mark_read(&st, &card);
            let card = st.store.lock().unwrap().mark_read(&p.id)?;
            let card = st.decorate(card);
            st.emit_card(card.clone());
            to_value(CardResult { card })
        }

        Method::SessionAttach(p) => {
            st.store.lock().unwrap().get_card(&p.card_id)?;
            st.touched.touch(&p.card_id);
            let conn = ctx.conn.clone();
            let sid = p.card_id.clone();
            let sink: attach::Sink =
                Arc::new(move |data: &[u8]| conn.send(Message::binary(encode_term_frame(&sid, data))));
            let reply_conn = ctx.conn.clone();
            st.hub
                .attach(&p.card_id, ctx.conn_id, p.cols, p.rows, p.watch, sink, move |cols, rows| {
                    let r = serde_json::to_value(SizeResult { cols, rows }).expect("serialise");
                    send_res(&reply_conn, ResponseFrame::ok(req_id, r));
                })
                .await?;
            Ok(Reply::Sent)
        }

        Method::SessionDetach(p) => {
            st.touched.touch(&p.card_id);
            if st.hub.detach(&p.card_id, ctx.conn_id) && st.hub.subscriber_count(&p.card_id) == 0 {
                st.scrollback.save(&st, &p.card_id).await;
            }
            to_value(Empty {})
        }

        Method::SessionKill(p) => {
            st.store.lock().unwrap().get_card(&p.card_id)?;
            if !st.backend.exists(&p.card_id).await {
                return Err(err(ErrorCode::NotFound, "no live session"));
            }
            st.hub.drop_session(&p.card_id);
            st.mark_killed(&p.card_id);
            st.backend.kill(&p.card_id).await?;
            st.reconcile().await;
            to_value(Empty {})
        }

        Method::TermWrite(p) => {
            if !st.has_session(&p.card_id) {
                st.store.lock().unwrap().get_card(&p.card_id)?;
                return Err(err(ErrorCode::NotFound, "no live session"));
            }
            st.touched.touch(&p.card_id);
            if p.raw {
                st.backend.send_input(&p.card_id, p.data.as_bytes()).await?;
            } else {
                st.hub.write(&p.card_id, p.data.as_bytes()).await?;
            }
            to_value(Empty {})
        }

        Method::TermResize(p) => {
            let (cols, rows) = st.hub.resize(&p.card_id, ctx.conn_id, p.cols, p.rows)?;
            to_value(SizeResult { cols, rows })
        }

        Method::ApprovalAnswer(p) => {
            let card = st.store.lock().unwrap().get_card(&p.card_id)?;
            let via = st.agents.answer(&st, &card, &p).await?;
            to_value(ApprovalAnswerResult { via: via.into() })
        }

        Method::ChatPage(p) => {
            let card = st.store.lock().unwrap().get_card(&p.card_id)?;
            to_value(st.agents.chat_page(&st, ctx.conn_id, &card, &p).await?)
        }

        Method::ChatUnsubscribe(p) => {
            st.agents.chat_unsubscribe(&p.card_id, ctx.conn_id);
            to_value(Empty {})
        }

        Method::UsageGet(_) => {
            let (usage, accounts) = st.agents.usage();
            to_value(UsageResult { usage, accounts })
        }

        Method::CardGet(p) => {
            let card = st.store.lock().unwrap().get_card(&p.id)?;
            to_value(CardResult { card: st.decorate(card) })
        }

        Method::StatsSubscribe(p) => {
            let interval_ms = st.stats.subscribe(ctx.conn_id, p.interval_ms);
            to_value(StatsSubscribeResult { interval_ms })
        }

        Method::StatsUnsubscribe(_) => {
            st.stats.unsubscribe(ctx.conn_id);
            to_value(Empty {})
        }

        Method::SettingsGet(_) => to_value(SettingsResult { settings: st.settings() }),

        Method::SettingsUpdate(p) => {
            let mut s = st.settings();
            if let Some(v) = p.keep_awake {
                s.keep_awake = v;
            }
            if let Some(v) = p.auto_restore {
                s.auto_restore = v;
            }
            if let Some(v) = p.clipboard_passthrough {
                s.clipboard_passthrough = v;
                st.backend.set_clipboard(v).await;
            }
            if let Some(v) = p.idle_stop_minutes {
                // Up to a year; 0 switches it off.
                if v > 525_600 {
                    return Err(invalid("idleStopMinutes must be at most 525600"));
                }
                s.idle_stop_minutes = v;
            }
            if let Some(v) = p.silence_seconds {
                if !(1..=600).contains(&v) {
                    return Err(invalid("silenceSeconds must be between 1 and 600"));
                }
                s.silence_seconds = v;
            }
            st.set_settings(s.clone());
            to_value(SettingsResult { settings: s })
        }

        Method::LinkAdd(p) => {
            crate::context::add_link(&st, &p.a, &p.b)?;
            to_value(Empty {})
        }

        Method::LinkRemove(p) => {
            crate::context::remove_link(&st, &p.a, &p.b)?;
            to_value(Empty {})
        }

        Method::ContextList(p) => to_value(ContextListResult { cards: crate::context::list(&st, &p.card_id)? }),

        Method::ContextRead(p) => to_value(TextResult { text: crate::context::read(&st, &p).await? }),

        Method::MessageSend(p) => to_value(MessageResult { message: crate::context::send(&st, &p)? }),

        Method::MessageCancel(p) => {
            crate::context::cancel(&st, &p.id)?;
            to_value(Empty {})
        }

        Method::DaemonStatus(_) => to_value(st.status()),

        Method::CommentList(p) => {
            let store = st.store.lock().unwrap();
            store.get_card(&p.card_id)?;
            to_value(CommentListResult { comments: store.list_comments(&p.card_id)? })
        }

        Method::CommentAdd(p) => to_value(CommentResult { comment: crate::context::add_comment(&st, &p)? }),

        Method::CommentDelete(p) => {
            crate::context::delete_comment(&st, &p.id)?;
            to_value(Empty {})
        }

        Method::CardHandoff(p) => to_value(CardResult { card: crate::context::handoff(&st, &p).await? }),

        Method::CardAiRename(p) => to_value(CardResult { card: crate::rename::start(&st, &p.id).await? }),

        Method::AgentList(_) => {
            let installed = st.installed_agents.get_or_init(|| find_installed(st.backend.shell().to_string())).await;
            let mut list = agents::list(|cmd| installed.contains_key(cmd));
            for a in &mut list {
                a.capabilities = Some(st.agents.capabilities(&a.id));
            }
            to_value(AgentListResult { agents: list })
        }
        Method::AgentAccounts(p) => {
            if let Some(accounts) = st.accounts.get(p.refresh) {
                return to_value(AgentAccountsResult { accounts });
            }
            let installed = st.installed_agents.get_or_init(|| find_installed(st.backend.shell().to_string())).await;
            let accounts = crate::accounts::check(installed).await;
            st.accounts.put(accounts.clone());
            to_value(AgentAccountsResult { accounts })
        }
    }
}

enum Restricted {
    /// Run this (possibly rewritten) call.
    Call(Method),
    /// Already answered.
    Done(Value),
}

fn refuse(what: &str) -> muster_proto::ErrorBody {
    err(ErrorCode::Unauthorized, format!("{what} is not allowed with a card token"))
}

/// The card role: an agent may read context of linked cards, message them, keep its own
/// notes, move and hand off its own card, and nothing else. The caller's card always
/// comes from its token, never from a parameter.
fn restrict(st: &State, me: &str, hash: &str, m: Method) -> Result<Restricted> {
    if st.store.lock().unwrap().card_for_token_hash(hash)?.as_deref() != Some(me) {
        return Err(err(ErrorCode::Unauthorized, "this card's session has ended; its token is no longer valid"));
    }
    let linked = |other: &str| -> Result<bool> { Ok(other == me || st.store.lock().unwrap().is_linked(me, other)?) };
    let name = m.name();
    let m = match m {
        Method::Hello(p) => Method::Hello(p),
        Method::ContextList(_) => Method::ContextList(ContextListParams { card_id: me.to_string() }),
        Method::ContextRead(mut p) => {
            if !linked(&p.card_id)? {
                return Err(refuse("reading a card that is not linked"));
            }
            p.from_card_id = Some(me.to_string());
            Method::ContextRead(p)
        }
        Method::MessageSend(mut p) => {
            if p.card_id == me || !linked(&p.card_id)? {
                return Err(refuse("messaging a card that is not linked"));
            }
            p.from_card_id = Some(me.to_string());
            Method::MessageSend(p)
        }
        Method::CardGet(p) if linked(&p.id)? => Method::CardGet(p),
        Method::CardMove(p) if p.id == me => Method::CardMove(p),
        Method::CardHandoff(p) if p.id == me => Method::CardHandoff(p),
        // Comments: read and write on the caller's own card and on linked cards, always
        // signed as the caller. Deleting is for the user.
        Method::CommentList(p) => {
            if !linked(&p.card_id)? {
                return Err(refuse("reading comments of a card that is not linked"));
            }
            Method::CommentList(p)
        }
        Method::CommentAdd(mut p) => {
            if !linked(&p.card_id)? {
                return Err(refuse("commenting on a card that is not linked"));
            }
            p.from_card_id = Some(me.to_string());
            Method::CommentAdd(p)
        }
        Method::CardUpdate(mut p) if p.id == me => {
            if p.title.is_some()
                || p.prompt.is_some()
                || p.priority.is_some()
                || p.due_at.is_some()
                || p.title_auto.is_some()
            {
                return Err(refuse("changing anything but the card's notes"));
            }
            // The notes are signed as this card's agent.
            p.from_card_id = Some(me.to_string());
            Method::CardUpdate(p)
        }
        Method::BoardGet(p) => {
            // Only the caller's own project, showing only the caller and linked cards.
            let store = st.store.lock().unwrap();
            let mine = store.get_card(me)?;
            if p.project_id != mine.project_id {
                return Err(refuse("reading another project"));
            }
            let project = store.get_project(&p.project_id)?;
            let columns = store.list_columns(&p.project_id)?;
            let visible: Vec<String> = std::iter::once(me.to_string()).chain(store.linked(me)?).collect();
            let cards = store
                .list_cards(&p.project_id)?
                .into_iter()
                .filter(|c| visible.contains(&c.id))
                .map(|c| st.decorate(c))
                .collect();
            let links = store.project_links(&p.project_id)?.into_iter().filter(|l| l.a == me || l.b == me).collect();
            let v = serde_json::to_value(BoardResult { project, columns, cards, links }).expect("serialise");
            return Ok(Restricted::Done(v));
        }
        // `project.summary` is among the refused: it names every card of the project,
        // and a card may only see itself and the cards linked to it.
        _ => return Err(refuse(name)),
    };
    Ok(Restricted::Call(m))
}

/// Starts (or resumes) the card's session and marks it running. Agents with an
/// adapter get a Muster-chosen session id (stored as `agentSessionId`) and hooks; a card
/// that already has a session id is resumed.
pub(crate) async fn start_card(st: &Arc<State>, card: Card) -> Result<Card> {
    let hook_sock = st.home.join("hook.sock");
    let mut env: Vec<(String, String)> = vec![
        ("MUSTER_CARD_ID".into(), card.id.clone()),
        ("MUSTER_AGENT_ID".into(), card.agent_id.clone()),
        ("MUSTER_HOOK_SOCK".into(), hook_sock.to_string_lossy().into_owned()),
        ("COLORTERM".into(), "truecolor".into()),
        // Where the CLI finds the daemon's port. The master token stays out of cards:
        // the card gets its own token, which only allows acting for this card.
        ("MUSTER_DAEMON_FILE".into(), st.home.join("daemon.json").to_string_lossy().into_owned()),
    ];
    let card_token = hex(&muster_core::util::random_bytes(32));
    env.push(("MUSTER_CARD_TOKEN".into(), card_token.clone()));
    if let Some(cli) = crate::cli_path() {
        env.push(("MUSTER_CLI".into(), cli.to_string_lossy().into_owned()));
    }
    let (argv, session_id) = match st.agents.launch_command(&card)? {
        Some((argv, extra_env, sid)) => {
            for (k, v) in extra_env {
                env.retain(|(ek, _)| *ek != k);
                env.push((k, v));
            }
            (Some(argv), sid)
        }
        None => (
            agents::launch_command(&card.agent_id, card.prompt.as_deref(), card.agent_session_id.as_deref())
                .map_err(invalid)?,
            card.agent_session_id.clone(),
        ),
    };
    // A worktree card of a project with a setup script runs it first, in the session,
    // on its first start and again after a failed attempt.
    let project = st.store.lock().unwrap().get_project(&card.project_id)?;
    let setup_failed = card.setup.as_ref().is_some_and(|s| s.state == SetupState::Failed);
    let setup = project
        .worktree_setup
        .clone()
        .filter(|_| card.branch.is_some() && (card.status == CardStatus::Draft || setup_failed));
    // A card whose session vanished (reboot, tmux died) gets its saved scrollback
    // replayed before the agent resumes.
    let replay = card.restorable && setup.is_none();
    let argv = match &setup {
        None if replay => {
            let saved = crate::scrollback::path(st, &card.id);
            if saved.is_file() {
                Some(vec!["/bin/sh".to_string(), write_restore_wrapper(st, &card, &saved, argv.as_deref())?])
            } else {
                argv
            }
        }
        None => argv,
        Some(script) => {
            env.push(("MUSTER_PROJECT_PATH".into(), project.path.clone()));
            env.push(("MUSTER_WORKTREE_PATH".into(), card.cwd.clone()));
            env.push(("MUSTER_BRANCH".into(), card.branch.clone().unwrap_or_default()));
            // TMUX is not set in cards, so the wrapper names its window explicitly.
            env.push(("MUSTER_TMUX_TARGET".into(), format!("={}:", muster_core::tmux::session_name(&card.id))));
            let wrapper = write_setup_wrapper(st, &card, script, argv.as_deref())?;
            Some(vec!["/bin/sh".to_string(), wrapper])
        }
    };
    let spec = SessionSpec {
        card_id: card.id.clone(),
        cwd: card.cwd.clone(),
        argv,
        env,
        cols: attach::DEFAULT_SIZE.0,
        rows: attach::DEFAULT_SIZE.1,
    };
    if session_id != card.agent_session_id {
        st.store.lock().unwrap().set_agent_session_id(&card.id, session_id.as_deref())?;
    }
    st.store.lock().unwrap().set_card_token_hash(&card.id, Some(&hex(&sha256(card_token.as_bytes()))))?;
    if card.agent_id == "claude" {
        check_claude_permission_hooks(st);
    }
    let creating = st.hold_reconcile().await;
    st.backend.create(&spec).await?;
    if st.settings().clipboard_passthrough {
        // A new tmux server starts from the config file, where it is off.
        st.backend.set_clipboard(true).await;
    }
    st.mark_live(&card.id);
    log!(
        "started session for card {} ({}{})",
        card.id,
        card.agent_id,
        if card.agent_session_id.is_some() { ", resumed" } else { "" }
    );
    if card.restorable {
        st.store.lock().unwrap().set_restorable(&card.id, false)?;
    }
    if card.idle_stopped_at.is_some() || card.idle_kept.is_some() {
        st.store.lock().unwrap().set_idle_stopped(&card.id, None)?;
    }
    let card = st.store.lock().unwrap().set_status(&card.id, CardStatus::Running)?;
    drop(creating);
    if setup.is_some() {
        log!("card {}: running worktree setup", card.id);
        let info = SetupInfo { state: SetupState::Running, exit_code: None };
        st.store.lock().unwrap().set_setup(&card.id, Some(&info))?;
    } else {
        if card.setup.is_some() {
            st.store.lock().unwrap().set_setup(&card.id, None)?;
        }
        st.agents.on_started(st, &card);
    }
    let card = st.store.lock().unwrap().get_card(&card.id)?;
    Ok(card)
}

fn write_restore_wrapper(st: &State, card: &Card, saved: &Path, argv: Option<&[String]>) -> Result<String> {
    let dir = st.home.join("setup");
    std::fs::create_dir_all(&dir).map_err(muster_core::util::internal)?;
    let shell = shell_quote(st.backend.shell());
    let next = match argv {
        Some(argv) => argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "),
        None => format!("{shell} -l"),
    };
    let wrapper = format!(
        "#!/bin/sh\ncat {saved}\nprintf '\\r\\n\\033[2m[muster] restored after the session was lost\\033[0m\\r\\n'\nexec {next}\n",
        saved = shell_quote(&saved.to_string_lossy()),
    );
    let path = dir.join(format!("{}.restore.sh", card.id));
    std::fs::write(&path, wrapper).map_err(muster_core::util::internal)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Writes the setup script and the wrapper that runs it before the agent; returns the
/// wrapper's path. The wrapper reports the exit code by renaming its tmux window
/// (`muster-setup:<code>`), which the tmux control client sees at once.
fn write_setup_wrapper(st: &State, card: &Card, script: &str, argv: Option<&[String]>) -> Result<String> {
    let dir = st.home.join("setup");
    std::fs::create_dir_all(&dir).map_err(muster_core::util::internal)?;
    let script_path = dir.join(format!("{}.sh", card.id));
    std::fs::write(&script_path, script).map_err(muster_core::util::internal)?;
    let shell = shell_quote(st.backend.shell());
    let next = match argv {
        Some(argv) => argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" "),
        None => format!("{shell} -l"),
    };
    let wrapper = format!(
        "#!/bin/sh\n\
         printf '\\033[2m[muster] worktree setup in %s\\033[0m\\n' \"$MUSTER_WORKTREE_PATH\"\n\
         {shell} {script}\n\
         code=$?\n\
         {tmux} -L {socket} rename-window -t \"$MUSTER_TMUX_TARGET\" \"{mark}$code\" 2>/dev/null\n\
         if [ \"$code\" -eq 0 ]; then\n  exec {next}\nfi\n\
         printf '\\n[muster] worktree setup failed (exit %s). Fix it in this shell, then start the card again.\\n' \"$code\"\n\
         exec {shell} -l\n",
        script = shell_quote(&script_path.to_string_lossy()),
        tmux = shell_quote(&st.backend.tmux_path().to_string_lossy()),
        socket = shell_quote(st.backend.socket()),
        mark = muster_core::tmux::SETUP_MARK,
    );
    let wrapper_path = dir.join(format!("{}.run.sh", card.id));
    std::fs::write(&wrapper_path, wrapper).map_err(muster_core::util::internal)?;
    Ok(wrapper_path.to_string_lossy().into_owned())
}

/// Files the daemon keeps per card: scrollback, setup/restore wrappers, exit status.
pub(crate) fn card_files(st: &State, card_id: &str) -> Vec<std::path::PathBuf> {
    let setup = st.home.join("setup");
    vec![
        crate::scrollback::path(st, card_id),
        setup.join(format!("{card_id}.sh")),
        setup.join(format!("{card_id}.run.sh")),
        setup.join(format!("{card_id}.restore.sh")),
        st.home.join("exit").join(muster_core::tmux::session_name(card_id)),
    ]
}

fn remove_card_files(st: &State, card_id: &str) {
    for f in card_files(st, card_id) {
        let _ = std::fs::remove_file(f);
    }
}

/// Notes (read only, contents never logged) whether the user's own Claude Code settings
/// define PermissionRequest hooks, which may answer before Muster's.
fn check_claude_permission_hooks(st: &State) {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".claude")));
    let has = dir
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("hooks").and_then(|h| h.get("PermissionRequest")).cloned())
        .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    st.set_notice(
        "claude_permission_hooks",
        has.then(|| {
            (
                "info",
                "Your Claude Code settings define PermissionRequest hooks of their own; approvals from Muster may be \
                 pre-empted by other tools."
                    .to_string(),
            )
        }),
    );
}

/// The automatic title of a card created without title or prompt.
fn placeholder_title(agent_id: &str, label: &str, cwd: &str) -> String {
    if agent_id == "shell" {
        if let Some(name) = Path::new(cwd).file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    label.to_string()
}

/// Brings `lastActivityAt` of live cards without hooks up to date from tmux's own
/// record of the last output in each session (one tmux call; no polling).
async fn refresh_activity(st: &Arc<State>) {
    if st.live_count() == 0 {
        return;
    }
    let times = st.backend.activity_times().await;
    let cards = st.store.lock().unwrap().all_cards().unwrap_or_default();
    for card in cards {
        let Some(&t) = times.get(&card.id) else { continue };
        if st.agents.hooked(&card) || card.last_activity_at.is_some_and(|a| a >= t) {
            continue;
        }
        let _ = st.store.lock().unwrap().touch_activity(&card.id, t);
    }
}

/// Resolves which agent commands are on the login shell's PATH (run once).
/// Agent command name → its full path on the login shell's PATH.
async fn find_installed(shell: String) -> HashMap<String, PathBuf> {
    let run = tokio::process::Command::new(&shell)
        .args(["-l", "-i", "-c", "echo \"MUSTER-PATH:$PATH\""])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let path = match tokio::time::timeout(Duration::from_secs(10), run).await {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.trim().strip_prefix("MUSTER-PATH:").map(str::to_string)),
        _ => None,
    };
    let path = path.unwrap_or_else(|| {
        log!("could not read PATH from {shell}; using the daemon's own");
        std::env::var("PATH").unwrap_or_default()
    });
    // fish prints its PATH list space-separated.
    let dirs: Vec<&str> =
        if path.contains(':') { path.split(':').collect() } else { path.split_whitespace().collect() };
    agents::AGENTS
        .iter()
        .filter_map(|a| a.command)
        .filter_map(|cmd| {
            dirs.iter()
                .map(|d| Path::new(d).join(cmd))
                .find(|p| {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                })
                .map(|p| (cmd.to_string(), p))
        })
        .collect()
}
