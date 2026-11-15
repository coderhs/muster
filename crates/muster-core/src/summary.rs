//! What was worked on in a project during a time range, from the activity log.
//!
//! The log holds one row per status change (`{"from","to"}`), one when a card is
//! created, and one when the daemon stops an idle session. Working time is the time
//! a card spent `running`, clipped to the range: a card already running when the
//! range starts counts from the start, one still running at its end counts up to the
//! end (or to now, when the range reaches into the future).

use std::collections::HashMap;

use muster_proto::{
    CardStatus, ProjectSummaryResult, SummaryActive, SummaryCard, SummaryDay, SummaryOutcome, SummarySessionEnd,
    SummaryTotals,
};

/// A row of the activity log.
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRow {
    pub card_id: String,
    pub ts: i64,
    pub kind: String,
    pub payload: Option<String>,
}

/// The fields of a card the summary shows.
#[derive(Debug, Clone, PartialEq)]
pub struct CardInfo {
    pub id: String,
    pub title: String,
    pub agent_id: String,
    pub model: Option<String>,
    pub column_id: String,
    pub column_title: String,
    pub status: CardStatus,
    pub status_since: i64,
    pub idle_stopped_at: Option<i64>,
}

/// The log has no idle-stop row for stops made before it recorded them; a session that
/// exits this soon after the card's `idle_stopped_at` was stopped by the daemon.
const IDLE_STOP_WINDOW_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Event {
    Created,
    /// `from` is only needed for the first row of a card, to learn the state before it.
    Status(Option<CardStatus>, CardStatus),
    IdleStop,
}

fn parse(row: &ActivityRow) -> Option<Event> {
    match row.kind.as_str() {
        "created" => Some(Event::Created),
        "idle_stop" => Some(Event::IdleStop),
        "status" => {
            let v: serde_json::Value = serde_json::from_str(row.payload.as_deref()?).ok()?;
            let to = CardStatus::parse(v.get("to")?.as_str()?)?;
            Some(Event::Status(v.get("from").and_then(|f| f.as_str()).and_then(CardStatus::parse), to))
        }
        _ => None,
    }
}

/// Summarises `[from, to)` and each part that `splits` cuts it into.
///
/// `rows` are the project's activity rows before `to`, ordered by card and then by
/// time, reaching back to each card's last row before `from` (which tells the state
/// the card was in when the range began). `hasSession` is left `false` for the caller.
pub fn build(
    cards: &[CardInfo],
    rows: &[ActivityRow],
    from: i64,
    to: i64,
    splits: &[i64],
    now: i64,
) -> ProjectSummaryResult {
    let by_id: HashMap<&str, &CardInfo> = cards.iter().map(|c| (c.id.as_str(), c)).collect();
    let timelines: Vec<(&CardInfo, Vec<(i64, Event)>)> = rows
        .chunk_by(|a, b| a.card_id == b.card_id)
        .filter_map(|chunk| {
            let card = by_id.get(chunk[0].card_id.as_str())?;
            Some((*card, chunk.iter().filter_map(|r| Some((r.ts, parse(r)?))).collect()))
        })
        .collect();
    let range = |from: i64, to: i64| {
        let mut list: Vec<SummaryCard> =
            timelines.iter().filter_map(|(card, events)| slice(card, events, from, to, now)).collect();
        list.sort_by(|a, b| b.last_at.cmp(&a.last_at).then_with(|| a.id.cmp(&b.id)));
        let totals = totals(&list);
        (list, totals)
    };

    let mut cuts: Vec<i64> = splits.iter().copied().filter(|s| *s > from && *s < to).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut days = Vec::new();
    if !cuts.is_empty() {
        let starts = std::iter::once(from).chain(cuts.iter().copied());
        let ends = cuts.iter().copied().chain(std::iter::once(to));
        for (a, b) in starts.zip(ends) {
            let (cards, totals) = range(a, b);
            if !cards.is_empty() {
                days.push(SummaryDay { from: a, to: b, cards, totals });
            }
        }
        days.reverse();
    }

    let mut active: Vec<SummaryActive> = cards
        .iter()
        .filter(|c| matches!(c.status, CardStatus::Running | CardStatus::NeedsYou))
        .map(|c| SummaryActive {
            id: c.id.clone(),
            title: c.title.clone(),
            agent_id: c.agent_id.clone(),
            model: c.model.clone(),
            column_title: c.column_title.clone(),
            status: c.status,
            status_since: c.status_since,
            has_session: false,
        })
        .collect();
    // Cards waiting for the user first, the longest wait on top.
    active.sort_by(|a, b| {
        (a.status != CardStatus::NeedsYou, a.status_since, &a.id).cmp(&(
            b.status != CardStatus::NeedsYou,
            b.status_since,
            &b.id,
        ))
    });

    let (cards, totals) = range(from, to);
    ProjectSummaryResult { from, to, cards, totals, days, active }
}

fn totals(cards: &[SummaryCard]) -> SummaryTotals {
    let mut t = SummaryTotals::default();
    for c in cards {
        if c.runs > 0 {
            t.cards += 1;
            t.working_ms += c.working_ms;
            if c.outcome == SummaryOutcome::Finished {
                t.finished += 1;
            }
        }
        if c.created {
            t.created += 1;
        }
    }
    t
}

/// One card's part in `[from, to)`; `None` when it neither ran nor was created there.
fn slice(card: &CardInfo, events: &[(i64, Event)], from: i64, to: i64, now: i64) -> Option<SummaryCard> {
    let end = to.min(now);
    let mut state: Option<CardStatus> = None;
    // Start of the current `running` stretch; unknown (before the range) at first.
    let mut since = i64::MIN;
    // The current run already counts towards `runs`.
    let mut counted = false;
    let mut finished = false;
    let mut idle_stop = false;
    let mut session_end = None;
    let (mut working_ms, mut runs, mut first_at, mut last_at, mut created) = (0i64, 0u32, i64::MAX, i64::MIN, false);
    let mut add = |a: i64, b: i64, counted: &mut bool| {
        let (a, b) = (a.max(from), b.min(end));
        if b > a {
            working_ms += b - a;
            if !*counted {
                runs += 1;
                *counted = true;
            }
            first_at = first_at.min(a);
            last_at = last_at.max(b);
        }
    };
    let mut created_at = None;
    for &(ts, event) in events.iter().take_while(|(ts, _)| *ts < to) {
        match event {
            Event::Created => {
                state = Some(CardStatus::Draft);
                if ts >= from {
                    created = true;
                    created_at = Some(ts);
                }
            }
            Event::IdleStop => idle_stop = true,
            Event::Status(before, to_status) => {
                let prev = state.or(before);
                if prev == Some(CardStatus::Running) {
                    add(since, ts, &mut counted);
                }
                match to_status {
                    CardStatus::Running => {
                        since = ts;
                        // Answering a question or an approval continues the run.
                        if prev != Some(CardStatus::NeedsYou) {
                            counted = false;
                        }
                        (finished, idle_stop, session_end) = (false, false, None);
                    }
                    CardStatus::Done => finished = true,
                    CardStatus::Exited if ts >= from => {
                        let by_daemon =
                            idle_stop || card.idle_stopped_at.is_some_and(|t| t <= ts && ts - t < IDLE_STOP_WINDOW_MS);
                        session_end =
                            Some(if by_daemon { SummarySessionEnd::IdleStopped } else { SummarySessionEnd::Exited });
                    }
                    _ => {}
                }
                state = Some(to_status);
            }
        }
    }
    if state == Some(CardStatus::Running) {
        add(since, end, &mut counted);
    }
    if let Some(ts) = created_at {
        first_at = first_at.min(ts);
        last_at = last_at.max(ts);
    }
    if runs == 0 && !created {
        return None;
    }
    let outcome = match state {
        Some(CardStatus::Running) => SummaryOutcome::Working,
        Some(CardStatus::NeedsYou) => SummaryOutcome::NeedsYou,
        _ if runs == 0 => SummaryOutcome::NotStarted,
        _ if finished => SummaryOutcome::Finished,
        _ => SummaryOutcome::Idle,
    };
    Some(SummaryCard {
        id: card.id.clone(),
        title: card.title.clone(),
        agent_id: card.agent_id.clone(),
        model: card.model.clone(),
        column_id: card.column_id.clone(),
        column_title: card.column_title.clone(),
        status: card.status,
        has_session: false,
        first_at,
        last_at,
        working_ms,
        runs,
        outcome,
        session_end,
        created,
        summary: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const MIN: i64 = 60_000;
    /// A day far from zero, so that times before it stay positive.
    const DAY0: i64 = 1_000 * 24 * HOUR;
    const DAY1: i64 = DAY0 + 24 * HOUR;
    const LATER: i64 = DAY0 + 100 * 24 * HOUR;

    fn card(id: &str, status: CardStatus) -> CardInfo {
        CardInfo {
            id: id.into(),
            title: format!("card {id}"),
            agent_id: "claude".into(),
            model: Some("Opus".into()),
            column_id: "col".into(),
            column_title: "In Progress".into(),
            status,
            status_since: DAY0,
            idle_stopped_at: None,
        }
    }

    fn created(id: &str, ts: i64) -> ActivityRow {
        ActivityRow { card_id: id.into(), ts, kind: "created".into(), payload: None }
    }

    fn status(id: &str, ts: i64, from: &str, to: &str) -> ActivityRow {
        let payload = Some(format!(r#"{{"from":"{from}","to":"{to}"}}"#));
        ActivityRow { card_id: id.into(), ts, kind: "status".into(), payload }
    }

    fn idle_stop(id: &str, ts: i64) -> ActivityRow {
        ActivityRow { card_id: id.into(), ts, kind: "idle_stop".into(), payload: None }
    }

    fn day0(cards: &[CardInfo], rows: &[ActivityRow]) -> ProjectSummaryResult {
        build(cards, rows, DAY0, DAY1, &[], LATER)
    }

    #[test]
    fn created_and_finished_inside_the_range() {
        let rows = [
            created("a", DAY0 + HOUR),
            status("a", DAY0 + HOUR + 1000, "draft", "running"),
            status("a", DAY0 + HOUR + 10 * MIN + 1000, "running", "done"),
            status("a", DAY0 + 2 * HOUR, "done", "idle"),
        ];
        let r = day0(&[card("a", CardStatus::Idle)], &rows);
        let c = &r.cards[0];
        assert_eq!((c.working_ms, c.runs, c.created), (10 * MIN, 1, true));
        assert_eq!((c.outcome, c.session_end), (SummaryOutcome::Finished, None));
        // Reading the finished card is not work: the last activity is the end of the run.
        assert_eq!((c.first_at, c.last_at), (DAY0 + HOUR, DAY0 + HOUR + 10 * MIN + 1000));
        assert_eq!(r.totals, SummaryTotals { cards: 1, working_ms: 10 * MIN, finished: 1, created: 1 });
        assert!(r.days.is_empty() && r.active.is_empty());
    }

    #[test]
    fn range_cuts_a_running_period_at_both_ends() {
        // Started an hour before the range, still running after it.
        let rows = [status("a", DAY0 - HOUR, "idle", "running"), status("a", DAY1 + HOUR, "running", "done")];
        let r = build(&[card("a", CardStatus::Done)], &rows[..1], DAY0, DAY1, &[], LATER);
        let c = &r.cards[0];
        assert_eq!((c.working_ms, c.runs, c.outcome), (24 * HOUR, 1, SummaryOutcome::Working));
        assert_eq!((c.first_at, c.last_at, c.created), (DAY0, DAY1, false));

        // The day after sees the last hour of the same run, and its end.
        let r = build(&[card("a", CardStatus::Done)], &rows, DAY1, DAY1 + 24 * HOUR, &[], LATER);
        let c = &r.cards[0];
        assert_eq!((c.working_ms, c.runs, c.outcome), (HOUR, 1, SummaryOutcome::Finished));

        // Without the row before the range, the first row's `from` tells the state.
        let r = build(&[card("a", CardStatus::Done)], &rows[1..], DAY1, DAY1 + 24 * HOUR, &[], LATER);
        assert_eq!(r.cards[0].working_ms, HOUR);
    }

    #[test]
    fn a_run_in_progress_counts_up_to_now() {
        let rows = [status("a", DAY0 + HOUR, "idle", "running")];
        let r = build(&[card("a", CardStatus::Running)], &rows, DAY0, DAY1, &[], DAY0 + 3 * HOUR);
        let c = &r.cards[0];
        assert_eq!((c.working_ms, c.last_at, c.outcome), (2 * HOUR, DAY0 + 3 * HOUR, SummaryOutcome::Working));
        assert_eq!(r.totals.finished, 0);
        assert_eq!(r.active.len(), 1);
        // A range that has not begun yet is empty.
        assert!(build(&[card("a", CardStatus::Running)], &rows, DAY1, DAY1 + HOUR, &[], DAY0 + 3 * HOUR)
            .cards
            .is_empty());
    }

    #[test]
    fn several_runs_in_a_day() {
        let rows = [
            status("a", DAY0 + HOUR, "idle", "running"),
            status("a", DAY0 + HOUR + 5 * MIN, "running", "done"),
            status("a", DAY0 + 2 * HOUR, "done", "running"),
            // A question in the middle: the same run goes on after the answer.
            status("a", DAY0 + 2 * HOUR + 10 * MIN, "running", "needs_you"),
            status("a", DAY0 + 3 * HOUR, "needs_you", "running"),
            status("a", DAY0 + 3 * HOUR + 20 * MIN, "running", "done"),
            status("a", DAY0 + 5 * HOUR, "done", "idle"),
            status("a", DAY0 + 6 * HOUR, "idle", "running"),
            status("a", DAY0 + 6 * HOUR + MIN, "running", "idle"),
        ];
        let r = day0(&[card("a", CardStatus::Idle)], &rows);
        let c = &r.cards[0];
        assert_eq!((c.working_ms, c.runs), (36 * MIN, 3));
        // The last run was interrupted, not finished.
        assert_eq!(c.outcome, SummaryOutcome::Idle);
        assert_eq!(r.totals.finished, 0);

        // Waiting for the user at the end of the range.
        let r = build(&[card("a", CardStatus::Idle)], &rows, DAY0, DAY0 + 2 * HOUR + 30 * MIN, &[], LATER);
        assert_eq!((r.cards[0].outcome, r.cards[0].runs), (SummaryOutcome::NeedsYou, 2));
        // A range holding only the second half of the interrupted run still counts it.
        let r = build(&[card("a", CardStatus::Idle)], &rows, DAY0 + 3 * HOUR, DAY0 + 4 * HOUR, &[], LATER);
        assert_eq!((r.cards[0].working_ms, r.cards[0].runs), (20 * MIN, 1));
    }

    #[test]
    fn empty_range_and_untouched_cards() {
        let rows = [
            created("a", DAY0 - 2 * HOUR),
            status("a", DAY0 - 2 * HOUR, "draft", "running"),
            status("a", DAY0 - HOUR, "running", "done"),
            // Reading it and the session ending are not work.
            status("a", DAY0 + HOUR, "done", "idle"),
            status("a", DAY0 + 2 * HOUR, "idle", "exited"),
        ];
        let r = day0(&[card("a", CardStatus::Exited), card("b", CardStatus::Draft)], &rows);
        assert!(r.cards.is_empty());
        assert_eq!(r.totals, SummaryTotals::default());
        assert!(day0(&[], &[]).cards.is_empty());
    }

    #[test]
    fn drafts_and_session_ends() {
        let mut stopped = card("b", CardStatus::Exited);
        stopped.idle_stopped_at = Some(DAY0 + 8 * HOUR);
        let rows = [
            created("a", DAY0 + HOUR),
            status("b", DAY0 + HOUR, "idle", "running"),
            status("b", DAY0 + 2 * HOUR, "running", "done"),
            status("b", DAY0 + 8 * HOUR + 2000, "done", "exited"),
            status("c", DAY0 + HOUR, "idle", "running"),
            status("c", DAY0 + 2 * HOUR, "running", "exited"),
            status("d", DAY0 + HOUR, "exited", "running"),
            status("d", DAY0 + 2 * HOUR, "running", "idle"),
            idle_stop("d", DAY0 + 5 * HOUR),
            status("d", DAY0 + 5 * HOUR + 3000, "idle", "exited"),
        ];
        // `d` runs again now, so only the log remembers that it was stopped.
        let cards = [card("a", CardStatus::Draft), stopped, card("c", CardStatus::Exited), card("d", CardStatus::Idle)];
        let r = day0(&cards, &rows);
        let get = |id: &str| r.cards.iter().find(|c| c.id == id).unwrap();
        assert_eq!((get("a").outcome, get("a").runs, get("a").created), (SummaryOutcome::NotStarted, 0, true));
        assert_eq!(
            (get("b").outcome, get("b").session_end),
            (SummaryOutcome::Finished, Some(SummarySessionEnd::IdleStopped))
        );
        assert_eq!((get("c").outcome, get("c").session_end), (SummaryOutcome::Idle, Some(SummarySessionEnd::Exited)));
        assert_eq!(get("d").session_end, Some(SummarySessionEnd::IdleStopped));
        // Drafts count as created, not as worked on.
        assert_eq!(r.totals, SummaryTotals { cards: 3, working_ms: 3 * HOUR, finished: 1, created: 1 });
        // Same time of last activity: ordered by id; the draft's creation is the oldest.
        assert_eq!(r.cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "c", "d", "a"]);
    }

    #[test]
    fn splits_give_one_part_per_day() {
        let rows = [
            status("a", DAY0 + 23 * HOUR, "idle", "running"),
            status("a", DAY1 + HOUR, "running", "done"),
            status("b", DAY1 + 30 * HOUR, "idle", "running"),
            status("b", DAY1 + 31 * HOUR, "running", "done"),
        ];
        let cards = [card("a", CardStatus::Done), card("b", CardStatus::NeedsYou), card("c", CardStatus::Running)];
        let to = DAY0 + 4 * 24 * HOUR;
        // Out of range and repeated cuts are ignored.
        let splits = [DAY1 + 48 * HOUR, DAY1, DAY1 + 24 * HOUR, DAY1, DAY0, to + HOUR];
        let r = build(&cards, &rows, DAY0, to, &splits, LATER);
        assert_eq!(r.totals, SummaryTotals { cards: 2, working_ms: 3 * HOUR, finished: 2, created: 0 });
        // Most recent day first; the two days without work are left out.
        let days: Vec<_> = r.days.iter().map(|d| (d.from, d.cards[0].id.as_str(), d.cards[0].working_ms)).collect();
        assert_eq!(days, [(DAY1 + 24 * HOUR, "b", HOUR), (DAY1, "a", HOUR), (DAY0, "a", HOUR)]);
        // The run that crosses midnight is one run on each day, and unfinished on the first.
        assert_eq!((r.days[2].cards[0].outcome, r.days[2].totals.finished), (SummaryOutcome::Working, 0));
        assert_eq!(r.days[1].cards[0].outcome, SummaryOutcome::Finished);
        assert_eq!(r.cards.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        // Active now: the card waiting for the user comes first.
        assert_eq!(r.active.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    }

    #[test]
    fn unknown_rows_are_ignored() {
        let rows = [
            ActivityRow { card_id: "a".into(), ts: DAY0 + 1, kind: "move".into(), payload: Some("{}".into()) },
            ActivityRow { card_id: "a".into(), ts: DAY0 + 2, kind: "status".into(), payload: Some("nope".into()) },
            status("a", DAY0 + HOUR, "idle", "running"),
            status("a", DAY0 + 2 * HOUR, "running", "done"),
            // A card that was deleted has no entry in `cards`.
            status("gone", DAY0 + HOUR, "idle", "running"),
        ];
        let r = day0(&[card("a", CardStatus::Done)], &rows);
        assert_eq!(r.cards.len(), 1);
        assert_eq!(r.cards[0].working_ms, HOUR);
    }
}
