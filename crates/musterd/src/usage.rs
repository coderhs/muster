//! Account usage limits, one entry per agent, merged from every card's reports.
//!
//! Claude Code puts the rate-limit headers of its own latest API response into every status
//! line refresh. A session that has been idle for hours keeps refreshing with what it saw
//! back then, and leaves a window out once its reset time has passed. Taking the newest
//! report as the truth therefore rolled the numbers back to an idle card's old values, or
//! dropped the 5-hour window altogether. Within one window the share used only grows and
//! the reset time stays put, which orders reports without caring which session spoke last.

use std::collections::BTreeMap;

use muster_proto::{LimitWindow, NamedWindow, UsageInfo};

/// Reports whose reset times are closer than this describe the same window. The shortest
/// window is five hours, so the next window always moves the reset by hours.
const SAME_WINDOW_MS: i64 = 30 * 60_000;

/// Meta key the merged accounts are saved under.
pub const META_KEY: &str = "usage.accounts";
/// Older daemons saved a single snapshot here.
pub const LEGACY_META_KEY: &str = "usage";

#[derive(Clone, Copy)]
enum Keep {
    Current,
    Observed,
    /// Same window: the larger share used, the newer reset time.
    Both,
}

fn decide(cur: Option<i64>, obs: Option<i64>) -> Keep {
    match (cur, obs) {
        (Some(c), Some(o)) if o > c + SAME_WINDOW_MS => Keep::Observed,
        (Some(c), Some(o)) if o < c - SAME_WINDOW_MS => Keep::Current,
        (Some(_), Some(_)) => Keep::Both,
        // Without reset times there is nothing to order by; the report is all there is.
        _ => Keep::Observed,
    }
}

fn live(resets_at: Option<i64>, now: i64) -> bool {
    resets_at.is_none_or(|r| r > now)
}

pub fn merge_window(cur: Option<&LimitWindow>, obs: Option<&LimitWindow>, now: i64) -> Option<LimitWindow> {
    let cur = cur.filter(|w| live(w.resets_at, now));
    let obs = obs.filter(|w| live(w.resets_at, now));
    match (cur, obs) {
        (None, o) => o.cloned(),
        // A report without this window says nothing about it (Claude Code leaves a window out
        // once its own copy has expired), so the current value stands.
        (Some(c), None) => Some(c.clone()),
        (Some(c), Some(o)) => Some(match decide(c.resets_at, o.resets_at) {
            Keep::Current => c.clone(),
            Keep::Observed => o.clone(),
            Keep::Both => LimitWindow { used_percent: c.used_percent.max(o.used_percent), resets_at: o.resets_at },
        }),
    }
}

fn merge_named(cur: &[NamedWindow], obs: &[NamedWindow], now: i64) -> Vec<NamedWindow> {
    let mut out: Vec<NamedWindow> = cur.iter().filter(|w| live(w.resets_at, now)).cloned().collect();
    for o in obs.iter().filter(|w| live(w.resets_at, now)) {
        match out.iter_mut().find(|c| c.name == o.name) {
            None => out.push(o.clone()),
            Some(c) => match decide(c.resets_at, o.resets_at) {
                Keep::Current => {}
                Keep::Observed => *c = o.clone(),
                Keep::Both => {
                    c.used_percent = c.used_percent.max(o.used_percent);
                    c.resets_at = o.resets_at;
                    c.window_minutes = o.window_minutes.or(c.window_minutes);
                }
            },
        }
    }
    out
}

/// `obs` folded into `cur` (both of the same agent).
pub fn merge(cur: Option<&UsageInfo>, obs: &UsageInfo, now: i64) -> UsageInfo {
    let Some(cur) = cur else { return obs.clone() };
    UsageInfo {
        five_hour: merge_window(cur.five_hour.as_ref(), obs.five_hour.as_ref(), now),
        seven_day: merge_window(cur.seven_day.as_ref(), obs.seven_day.as_ref(), now),
        windows: merge_named(&cur.windows, &obs.windows, now),
        agent_id: obs.agent_id.clone().or_else(|| cur.agent_id.clone()),
        updated_at: obs.updated_at.max(cur.updated_at),
    }
}

/// True when the numbers a client shows would differ.
fn same_numbers(a: &UsageInfo, b: &UsageInfo) -> bool {
    a.five_hour == b.five_hour && a.seven_day == b.seven_day && a.windows == b.windows
}

#[derive(Default)]
pub struct Accounts {
    by_agent: BTreeMap<String, UsageInfo>,
}

impl Accounts {
    /// From the saved list, or an older daemon's single snapshot.
    pub fn load(saved: Option<&str>, legacy: Option<&str>) -> Accounts {
        let list: Vec<UsageInfo> = saved
            .and_then(|j| serde_json::from_str(j).ok())
            .or_else(|| legacy.and_then(|j| serde_json::from_str::<UsageInfo>(j).ok()).map(|u| vec![u]))
            .unwrap_or_default();
        Accounts {
            by_agent: list.into_iter().map(|u| (u.agent_id.clone().unwrap_or_else(|| "claude".into()), u)).collect(),
        }
    }

    /// Folds in one report. Returns the merged entry when its numbers changed.
    pub fn observe(&mut self, agent_id: &str, obs: UsageInfo, now: i64) -> Option<UsageInfo> {
        let cur = self.by_agent.get(agent_id);
        let merged = merge(cur, &obs, now);
        let changed = cur.is_none_or(|c| !same_numbers(c, &merged));
        self.by_agent.insert(agent_id.to_string(), merged.clone());
        changed.then_some(merged)
    }

    pub fn all(&self) -> Vec<UsageInfo> {
        self.by_agent.values().cloned().collect()
    }

    /// The single snapshot older clients read: Claude Code's, else the newest.
    pub fn primary(&self) -> Option<UsageInfo> {
        self.by_agent.get("claude").or_else(|| self.by_agent.values().max_by_key(|u| u.updated_at)).cloned()
    }

    pub fn to_json(&self) -> Option<String> {
        serde_json::to_string(&self.all()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    fn w(pct: f64, resets: i64) -> LimitWindow {
        LimitWindow { used_percent: pct, resets_at: Some(resets) }
    }

    fn info(five: Option<LimitWindow>, week: Option<LimitWindow>, at: i64) -> UsageInfo {
        UsageInfo { five_hour: five, seven_day: week, windows: vec![], agent_id: Some("claude".into()), updated_at: at }
    }

    #[test]
    fn idle_session_cannot_roll_back_the_same_window() {
        let now = 10 * H;
        let cur = w(40.0, now + 2 * H);
        // An idle card replays what it saw earlier in this window.
        assert_eq!(merge_window(Some(&cur), Some(&w(12.0, now + 2 * H)), now), Some(cur.clone()));
        // A busy card reports more.
        assert_eq!(
            merge_window(Some(&cur), Some(&w(55.0, now + 2 * H + 1000)), now),
            Some(w(55.0, now + 2 * H + 1000))
        );
    }

    #[test]
    fn a_new_window_replaces_and_an_old_one_is_ignored() {
        let now = 10 * H;
        let cur = w(90.0, now + H);
        assert_eq!(merge_window(Some(&cur), Some(&w(3.0, now + 6 * H)), now), Some(w(3.0, now + 6 * H)));
        let newer = w(3.0, now + 6 * H);
        assert_eq!(merge_window(Some(&newer), Some(&w(90.0, now + H)), now), Some(newer.clone()));
    }

    #[test]
    fn expired_windows_are_dropped_and_missing_ones_kept() {
        let now = 10 * H;
        // The stored window has reset; an old report of it is no better.
        assert_eq!(merge_window(Some(&w(80.0, now - 1)), Some(&w(80.0, now - 1)), now), None);
        // A report without the 5-hour window keeps the one we have.
        let cur = info(Some(w(20.0, now + H)), Some(w(30.0, now + 50 * H)), 1);
        let obs = info(None, Some(w(31.0, now + 50 * H)), 2);
        let m = merge(Some(&cur), &obs, now);
        assert_eq!(m.five_hour, Some(w(20.0, now + H)));
        assert_eq!(m.seven_day, Some(w(31.0, now + 50 * H)));
        assert_eq!(m.updated_at, 2);
    }

    #[test]
    fn named_windows_merge_by_name() {
        let now = 10 * H;
        let nw = |name: &str, pct: f64, resets: i64| NamedWindow {
            name: name.into(),
            window_minutes: Some(300),
            used_percent: pct,
            resets_at: Some(resets),
        };
        let cur = vec![nw("primary", 50.0, now + H), nw("secondary", 10.0, now + 90 * H)];
        let obs = vec![nw("primary", 20.0, now + H)];
        let m = merge_named(&cur, &obs, now);
        assert_eq!(m, cur);
        let m = merge_named(&cur, &[nw("primary", 1.0, now + 5 * H)], now);
        assert_eq!(m[0].used_percent, 1.0);
    }

    #[test]
    fn accounts_keep_agents_apart() {
        let now = 10 * H;
        let mut a = Accounts::default();
        assert!(a.observe("claude", info(Some(w(20.0, now + H)), None, 1), now).is_some());
        let mut codex = info(Some(w(70.0, now + 3 * H)), None, 2);
        codex.agent_id = Some("codex".into());
        assert!(a.observe("codex", codex, now).is_some());
        // Unchanged numbers are not reported again.
        assert!(a.observe("claude", info(Some(w(20.0, now + H)), None, 3), now).is_none());
        assert_eq!(a.all().len(), 2);
        assert_eq!(a.primary().unwrap().agent_id.as_deref(), Some("claude"));
        let back = Accounts::load(a.to_json().as_deref(), None);
        assert_eq!(back.all(), a.all());
        let legacy = serde_json::to_string(&info(None, Some(w(29.0, now + 50 * H)), 1)).unwrap();
        assert_eq!(Accounts::load(None, Some(&legacy)).all().len(), 1);
    }
}
