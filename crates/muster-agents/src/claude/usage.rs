//! Status line input → [`UsageSnapshot`].
//!
//! Verified on 2.1.323: the status line command receives `context_window` (with
//! `context_window_size` and `current_usage`) on every refresh, and `rate_limits`
//! (`five_hour`, `seven_day`, each `{used_percentage, resets_at}` with `resets_at` in unix
//! seconds) once the session has had an API response, for subscription logins.

use serde_json::Value;

use crate::types::*;

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn u(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(Value::as_u64)
}

fn secs_to_ms(v: &Value, k: &str) -> Option<u64> {
    f(v, k).filter(|x| *x > 0.0).map(|x| (x * 1000.0) as u64)
}

fn window(v: Option<&Value>) -> Option<LimitWindow> {
    let v = v.filter(|v| v.is_object())?;
    Some(LimitWindow {
        used_percent: f(v, "used_percentage")?,
        resets_at_ms: secs_to_ms(v, "resets_at"),
    })
}

pub fn parse_statusline(p: &Value) -> UsageSnapshot {
    let context = p
        .get("context_window")
        .filter(|c| c.is_object())
        .and_then(|c| {
            let cur = c.get("current_usage").filter(|x| x.is_object())?;
            let used = [
                "input_tokens",
                "cache_creation_input_tokens",
                "cache_read_input_tokens",
            ]
            .iter()
            .filter_map(|k| u(cur, k))
            .sum();
            Some(ContextUsage {
                used_tokens: used,
                window_tokens: u(c, "context_window_size"),
                used_percent: f(c, "used_percentage"),
                source: UsageSource::Statusline,
            })
        });
    let rate_limits = p
        .get("rate_limits")
        .filter(|r| r.is_object())
        .map(|r| RateLimits {
            five_hour: window(r.get("five_hour")),
            seven_day: window(r.get("seven_day")),
            spend: r
                .get("spend_limit")
                .filter(|s| s.is_object())
                .and_then(|s| {
                    Some(SpendLimit {
                        used_percent: f(s, "used_percentage")?,
                        resets_at_ms: secs_to_ms(s, "resets_at"),
                        used_usd: f(s, "used_usd"),
                        limit_usd: f(s, "limit_usd"),
                        period: s.get("period").and_then(Value::as_str).map(str::to_string),
                    })
                }),
            windows: Vec::new(),
        });
    let model = p.get("model");
    UsageSnapshot {
        context,
        rate_limits,
        model_id: model
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        model_name: model
            .and_then(|m| m.get("display_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        session_name: p
            .get("session_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        cost_usd: p.get("cost").and_then(|c| f(c, "total_cost_usd")),
        agent_version: p.get("version").and_then(Value::as_str).map(str::to_string),
    }
}
