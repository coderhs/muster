//! Stops sessions that have been idle for longer than the user allows, leaving a card
//! that can be resumed. Off unless `idle_stop_minutes` is set.
//!
//! A session is a candidate when its agent can resume a conversation, it is neither
//! working nor waiting for the user, nobody has it open, and for the configured time
//! the agent has done nothing and the user has neither opened it nor typed into it. Before it is stopped it is checked for work that would die
//! with it: a job the agent scheduled inside the session, or a process it left running.
//! When in doubt the session stays, and the card says why.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use muster_core::procstats::{self, ProcTable};
use muster_core::tmux::SessionBackend;
use muster_core::util::now_ms;
use muster_proto::{Card, CardStatus};
use serde_json::Value;

use crate::log;
use crate::state::State;

fn env_ms(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// How often idle sessions are looked at.
fn period() -> Duration {
    Duration::from_millis(env_ms("MUSTER_IDLE_CHECK_MS", 5 * 60_000))
}

/// Length of one "minute" of the setting; shortened by the tests.
fn minute_ms() -> i64 {
    env_ms("MUSTER_IDLE_MINUTE_MS", 60_000) as i64
}

/// A recurring job a Claude Code session schedules expires by itself after this long.
const CRON_LIFETIME_MS: i64 = 7 * 24 * 3600 * 1000;
/// Processes an agent starts with itself (MCP servers and the like) appear within this
/// long of its own start; anything started later is work it was asked to do.
const HELPER_WINDOW_S: i64 = 120;
/// A process younger than this is a passing command (a status line, a hook), not a job.
const SETTLED_S: i64 = 60;

/// When the user last had each card open or typed into it (ms). Kept in memory only:
/// after a daemon restart the card's stored status times decide alone.
#[derive(Default)]
pub struct Touched(std::sync::Mutex<std::collections::HashMap<String, i64>>);

impl Touched {
    /// The user opened, closed or typed into the card's terminal.
    pub fn touch(&self, card_id: &str) {
        self.0.lock().unwrap().insert(card_id.to_string(), now_ms());
    }

    fn at(&self, card_id: &str) -> Option<i64> {
        self.0.lock().unwrap().get(card_id).copied()
    }
}

pub async fn run(st: Arc<State>) {
    // The first look comes soon after the daemon starts: how long a session has been
    // idle is stored with its card, so a restart does not start the wait again.
    tokio::time::sleep(period().min(Duration::from_secs(60))).await;
    loop {
        check(&st).await;
        tokio::time::sleep(period()).await;
    }
}

/// Tells clients about the card as it is now. The store lock is released before the
/// card is decorated and sent, which take it again.
fn emit(st: &State, card_id: &str) {
    let card = st.store.lock().unwrap().get_card(card_id);
    if let Ok(card) = card {
        st.emit_card(st.decorate(card));
    }
}

/// When the session was last used: the agent started or finished something, or the
/// user had the card open or typed into it. Output alone does not count. A terminal
/// repaints for reasons that are nobody's use of it (a resize, a redraw by the agent's
/// interface), and that must not start the wait again.
fn idle_since(card: &Card, touched: Option<i64>) -> i64 {
    card.status_since.max(card.last_activity_at.unwrap_or(0)).max(touched.unwrap_or(0))
}

/// Whether the card's session may be stopped at all, whatever its idle time.
fn eligible(st: &State, card: &Card) -> bool {
    matches!(card.status, CardStatus::Idle | CardStatus::Done)
        && !card.keep_running
        && card.agent_id != "shell"
        // Without a conversation to resume, stopping would lose the work.
        && card.agent_session_id.is_some()
        && st.agents.capabilities(&card.agent_id).resume
        && st.has_session(&card.id)
        && st.hub.subscriber_count(&card.id) == 0
}

async fn check(st: &Arc<State>) {
    let minutes = st.settings().idle_stop_minutes as i64;
    let Ok(cards) = st.store.lock().unwrap().all_cards() else { return };
    let forget = |card: &Card| {
        if card.idle_kept.is_some() && st.store.lock().unwrap().set_idle_kept(&card.id, None).unwrap_or(false) {
            emit(st, &card.id);
        }
    };
    if minutes == 0 {
        cards.iter().for_each(forget);
        return;
    }
    let now = now_ms();
    let mut due = Vec::new();
    for card in &cards {
        if eligible(st, card) && now - idle_since(card, st.touched.at(&card.id)) >= minutes * minute_ms() {
            due.push(card.clone());
        } else {
            forget(card);
        }
    }
    if due.is_empty() {
        return;
    }
    let panes = st.backend.pane_pids().await;
    // One read of the process table and of each transcript, off the async threads.
    let holds: Vec<Option<String>> = {
        let (st, due) = (st.clone(), due.clone());
        let checked = tokio::task::spawn_blocking(move || {
            let table = ProcTable::read();
            due.iter().map(|card| held_by(&st, card, &due, panes.get(&card.id).copied(), &table)).collect()
        });
        checked.await.unwrap_or_default()
    };
    if holds.len() != due.len() {
        return; // the check itself failed: stop nothing
    }
    for (card, hold) in due.iter().zip(holds) {
        match hold {
            Some(why) => {
                if st.store.lock().unwrap().set_idle_kept(&card.id, Some(&why)).unwrap_or(false) {
                    log!("card {}: idle but kept running: {why}", card.id);
                    emit(st, &card.id);
                }
            }
            None => stop(st, card).await,
        }
    }
}

/// What would be lost if the card's session stopped now, in words for the user.
fn held_by(st: &State, card: &Card, due: &[Card], pane: Option<i32>, table: &ProcTable) -> Option<String> {
    // The session's own start: a resumed session forgets the jobs of the one before.
    let Some(pane) = pane else { return Some("its process could not be found".into()) };
    let Some((root_start, _)) = procstats::started(pane) else { return Some("its process could not be found".into()) };
    let procs: Vec<(i64, String)> =
        table.tree(pane).into_iter().filter(|&p| p != pane).filter_map(procstats::started).collect();
    if let Some(name) = background_process(&procs, root_start, now_ms() / 1000) {
        return Some(format!("a process it started is still running ({name})"));
    }
    if card.agent_id != "claude" {
        return None;
    }
    let cwd = Path::new(&card.cwd);
    let transcript = card
        .agent_session_id
        .as_deref()
        .and_then(|s| st.agents.adapter("claude").and_then(|a| a.transcript_path(s, cwd)));
    match transcript.map(std::fs::read_to_string) {
        Some(Ok(text)) => {
            if let Some(why) = claude_schedule(text.lines(), root_start * 1000, now_ms()) {
                return Some(why);
            }
        }
        // The conversation cannot be read, so nothing can be ruled out.
        Some(Err(_)) | None => return Some("its conversation could not be read".into()),
    }
    // Tasks saved with the project run in whichever session is open there.
    let others = st.store.lock().unwrap().all_cards().unwrap_or_default();
    let alone = !others.iter().any(|o| {
        o.id != card.id
            && o.agent_id == "claude"
            && o.cwd == card.cwd
            && st.has_session(&o.id)
            && !due.iter().any(|d| d.id == o.id)
    });
    if alone && project_tasks(&cwd.join(".claude/scheduled_tasks.json")) {
        return Some("the project has scheduled tasks and this is its only session".into());
    }
    None
}

/// The name of a process that the agent was asked to run and that is still there:
/// started well after the session's own start, and not just now.
pub fn background_process(procs: &[(i64, String)], root_start: i64, now_s: i64) -> Option<String> {
    procs
        .iter()
        .find(|(start, _)| *start > root_start + HELPER_WINDOW_S && now_s - *start >= SETTLED_S)
        .map(|(_, name)| name.clone())
}

fn project_tasks(file: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(file) else { return false };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(a)) => !a.is_empty(),
        Ok(Value::Object(o)) => o.values().any(|v| v.as_array().is_some_and(|a| !a.is_empty())),
        // Unreadable: assume there may be tasks.
        _ => true,
    }
}

/// Whether a Claude Code session still has something scheduled, from its transcript.
/// Jobs made with CronCreate and wake-ups made with ScheduleWakeup live only in the
/// session's memory, so the tool calls are the only record. `since_ms` is when the
/// running session started: earlier calls belong to a session that has ended.
pub fn claude_schedule<'a>(lines: impl Iterator<Item = &'a str>, since_ms: i64, now: i64) -> Option<String> {
    let mut jobs = 0u32;
    let mut wake: Option<i64> = None;
    for line in lines {
        if !(line.contains("CronCreate") || line.contains("CronDelete") || line.contains("ScheduleWakeup")) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(at) = v.get("timestamp").and_then(Value::as_str).and_then(parse_iso_ms) else { continue };
        if at < since_ms {
            continue;
        }
        let Some(parts) = v.pointer("/message/content").and_then(Value::as_array) else { continue };
        for part in parts.iter().filter(|p| p["type"] == "tool_use") {
            let input = &part["input"];
            match part["name"].as_str() {
                Some("CronCreate") => {
                    let recurring = input["recurring"].as_bool().unwrap_or(true);
                    let until = if recurring {
                        at + CRON_LIFETIME_MS
                    } else {
                        // A one-shot job fires at its next match; ten minutes of grace
                        // for the scheduler's offset and the turn it starts.
                        input["cron"].as_str().and_then(|c| pinned_time(c, at)).unwrap_or(at + CRON_LIFETIME_MS)
                            + 600_000
                    };
                    if until > now {
                        jobs += 1;
                    }
                }
                Some("CronDelete") => jobs = jobs.saturating_sub(1),
                Some("ScheduleWakeup") => {
                    wake = match input["stop"].as_bool() {
                        Some(true) => None,
                        _ => input["delaySeconds"].as_f64().map(|s| at + (s * 1000.0) as i64),
                    };
                }
                _ => {}
            }
        }
    }
    if jobs > 0 {
        return Some("has a scheduled job".into());
    }
    wake.filter(|w| *w > now).map(|_| "has a wake-up scheduled".into())
}

/// `2026-11-14T08:17:43.679Z` as unix milliseconds.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let rest = &s[19..];
    let (frac, zone) = match rest.strip_prefix('.') {
        Some(r) => {
            let n = r.bytes().take_while(u8::is_ascii_digit).count();
            let ms = format!("{:0<3}", &r[..n.min(3)]).parse::<i64>().ok()?;
            (ms, &r[n..])
        }
        None => (0, rest),
    };
    let offset_min = match zone {
        "Z" | "" => 0,
        z if z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) => {
            let m = z[1..3].parse::<i64>().ok()? * 60 + z[4..6].parse::<i64>().ok()?;
            if z.starts_with('-') {
                -m
            } else {
                m
            }
        }
        _ => return None,
    };
    // Days from civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec) - offset_min * 60) * 1000 + frac)
}

/// When a cron expression pinned to one minute, hour, day and month ("30 14 28 2 *")
/// next matches after `after_ms`, in local time. `None` for any other expression.
pub fn pinned_time(cron: &str, after_ms: i64) -> Option<i64> {
    let f: Vec<&str> = cron.split_whitespace().collect();
    if f.len() != 5 {
        return None;
    }
    let (min, hour, dom, mon) =
        (f[0].parse::<i32>().ok()?, f[1].parse::<i32>().ok()?, f[2].parse::<i32>().ok()?, f[3].parse::<i32>().ok()?);
    let after_s = (after_ms / 1000) as libc::time_t;
    let mut now_tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&after_s, &mut now_tm) }.is_null() {
        return None;
    }
    // This year's occurrence, or next year's when it has passed.
    (0..2).find_map(|add| {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = now_tm.tm_year + add;
        tm.tm_mon = mon - 1;
        tm.tm_mday = dom;
        tm.tm_hour = hour;
        tm.tm_min = min;
        tm.tm_isdst = -1;
        let t = unsafe { libc::mktime(&mut tm) } as i64;
        (t >= after_s as i64 - 60).then_some(t * 1000)
    })
}

/// Ends the session: asks the agent to quit, then removes whatever is left.
async fn stop(st: &Arc<State>, card: &Card) {
    // Someone may have opened or used the card while it was being checked.
    let fresh = st.store.lock().unwrap().get_card(&card.id);
    let Ok(fresh) = fresh else { return };
    if !eligible(st, &fresh) || fresh.status_since != card.status_since {
        return;
    }
    log!("card {}: stopping the idle session ({})", card.id, card.agent_id);
    // Marked first: the session's end is then seen as a stop, not as a crash to restore.
    let _ = st.store.lock().unwrap().set_idle_stopped(&card.id, Some(now_ms()));
    st.mark_killed(&card.id);
    // Ctrl+C twice is "quit" in every supported agent; it lets them save their state.
    for _ in 0..2 {
        let _ = st.backend.send_input(&card.id, b"\x03").await;
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    for _ in 0..8 {
        if !st.backend.exists(&card.id).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    st.hub.drop_session(&card.id);
    if st.backend.exists(&card.id).await {
        let _ = st.backend.kill(&card.id).await;
    }
    st.reconcile().await;
    emit(st, &card.id);
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-14T08:00:00.000Z";

    fn at(iso: &str) -> i64 {
        parse_iso_ms(iso).unwrap()
    }

    fn call(ts: &str, name: &str, input: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"content":[{{"type":"tool_use","id":"t","name":"{name}","input":{input}}}]}}}}"#
        )
    }

    fn schedule(lines: &[String], since: &str, now: &str) -> Option<String> {
        claude_schedule(lines.iter().map(String::as_str), at(since), at(now))
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-11-14T08:17:43.679Z"), Some(1_794_644_263_679));
        assert_eq!(parse_iso_ms("2026-11-14T13:47:43+05:30"), Some(1_794_644_263_000));
        assert_eq!(parse_iso_ms("yesterday"), None);
    }

    #[test]
    fn a_recurring_job_holds_until_deleted_or_expired() {
        let create = call("2026-11-14T09:00:00.000Z", "CronCreate", r#"{"cron":"7 * * * *","prompt":"check"}"#);
        let delete = call("2026-11-14T10:00:00.000Z", "CronDelete", r#"{"id":"j1"}"#);
        assert_eq!(
            schedule(std::slice::from_ref(&create), T0, "2026-11-16T09:00:00Z").as_deref(),
            Some("has a scheduled job")
        );
        assert_eq!(schedule(&[create.clone(), delete], T0, "2026-11-16T09:00:00Z"), None);
        // Recurring jobs expire by themselves after seven days.
        assert_eq!(schedule(std::slice::from_ref(&create), T0, "2026-11-22T09:00:00Z"), None);
        // Made by an earlier run of the conversation: gone with that process.
        assert_eq!(schedule(&[create], "2026-11-15T00:00:00Z", "2026-11-16T09:00:00Z"), None);
    }

    #[test]
    fn a_wake_up_holds_until_it_is_due_or_stopped() {
        let wake =
            call("2026-11-14T09:00:00.000Z", "ScheduleWakeup", r#"{"delaySeconds":1800,"prompt":"p","reason":"r"}"#);
        let stop = call("2026-11-14T09:05:00.000Z", "ScheduleWakeup", r#"{"stop":true}"#);
        assert_eq!(
            schedule(std::slice::from_ref(&wake), T0, "2026-11-14T09:10:00Z").as_deref(),
            Some("has a wake-up scheduled")
        );
        assert_eq!(schedule(std::slice::from_ref(&wake), T0, "2026-11-14T09:40:00Z"), None);
        assert_eq!(schedule(&[wake, stop], T0, "2026-11-14T09:10:00Z"), None);
    }

    #[test]
    fn a_one_shot_job_holds_until_its_time() {
        // Unpinned one-shot: no time can be worked out, so it holds for the full lifetime.
        let vague =
            call("2026-11-14T09:00:00.000Z", "CronCreate", r#"{"cron":"*/5 * * * *","prompt":"p","recurring":false}"#);
        assert!(schedule(&[vague], T0, "2026-11-15T09:00:00Z").is_some());
        // Pinned: holds until that minute has passed. Worked out in local time, so the
        // test asks for the time back instead of naming one.
        let created = at("2026-03-01T00:00:00Z");
        let fire = pinned_time("30 14 20 6 *", created).unwrap();
        assert!(fire > created && fire < created + 366 * 86_400_000);
        let line =
            call("2026-03-01T00:00:00.000Z", "CronCreate", r#"{"cron":"30 14 20 6 *","prompt":"p","recurring":false}"#);
        let lines = [line];
        let run = |now: i64| claude_schedule(lines.iter().map(String::as_str), created - 1, now);
        assert!(run(fire - 60_000).is_some());
        assert!(run(fire + 3_600_000).is_none());
        // Already passed this year: next year's.
        assert!(pinned_time("0 9 1 1 *", created).unwrap() > created);
        assert_eq!(pinned_time("*/5 * * * *", created), None);
    }

    #[test]
    fn other_tool_calls_and_text_are_ignored() {
        let text = r#"{"type":"user","timestamp":"2026-11-14T09:00:00.000Z","message":{"content":"please CronCreate something"}}"#;
        let bash = call("2026-11-14T09:00:00.000Z", "Bash", r#"{"command":"echo CronCreate"}"#);
        assert_eq!(schedule(&[text.to_string(), bash, "not json CronCreate".into()], T0, "2026-11-14T09:10:00Z"), None);
    }

    #[test]
    fn helpers_are_not_background_work() {
        let root = 1_000_000;
        let now = root + 86_400;
        // An MCP server started with the agent, and a status line that just ran.
        let helpers = [(root + 1, "node".to_string()), (now - 5, "muster-hook".to_string())];
        assert_eq!(background_process(&helpers, root, now), None);
        // A dev server the agent started an hour in.
        let with_job = [(root + 1, "node".to_string()), (root + 3600, "vite".to_string())];
        assert_eq!(background_process(&with_job, root, now).as_deref(), Some("vite"));
    }
}
