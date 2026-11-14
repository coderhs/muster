//! Short, human model names for cards ("Opus 5.5", "gpt-5.5", "Gemini 2.5 Pro").

/// Claude model families, as they appear in model ids.
const CLAUDE_FAMILIES: &[&str] = &["opus", "sonnet", "haiku", "fable", "mythos"];

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// `claude-opus-5-5`, `claude-3-5-sonnet-20241022`, `us.anthropic.claude-sonnet-4-5-v1:0`,
/// `claude-4.5-sonnet-thinking` (Cursor) → `Opus 5.5`, `Sonnet 3.5`, `Sonnet 4.5`.
fn claude_label(id: &str) -> Option<String> {
    // Bedrock's `-v1:0` revision suffix is not part of the version.
    let tail = id[id.find("claude")?..].split(':').next().unwrap_or_default();
    let mut family = None;
    let mut version = Vec::new();
    for part in tail.split(['-', '.', '@']) {
        if family.is_none() && CLAUDE_FAMILIES.contains(&part) {
            family = Some(part);
        } else if !part.is_empty() && part.len() <= 2 && part.bytes().all(|b| b.is_ascii_digit()) {
            // Dates (20250929) and `v1` suffixes are not part of the version.
            version.push(part);
        }
    }
    let family = capitalize(family?);
    Some(if version.is_empty() { family } else { format!("{family} {}", version.join(".")) })
}

/// A short label for a model, preferring the agent's own display name. `None` when there
/// is nothing usable.
pub fn model_label(id: Option<&str>, display_name: Option<&str>) -> Option<String> {
    if let Some(d) = display_name.map(str::trim).filter(|d| !d.is_empty()) {
        // "Opus 5.5 (1M context)" → "Opus 5.5": the card has little room.
        let d = d.split(" (").next().unwrap_or(d).trim();
        return Some(d.chars().take(32).collect());
    }
    let id = id.map(str::trim).filter(|i| !i.is_empty())?;
    // `[1m]` and similar suffixes select a context size, not a model.
    let id = id.split('[').next().unwrap_or(id);
    // opencode names models `provider/model`.
    let id = id.rsplit('/').next().unwrap_or(id);
    let lower = id.to_ascii_lowercase();
    if let Some(l) = claude_label(&lower) {
        return Some(l);
    }
    if CLAUDE_FAMILIES.contains(&lower.as_str()) || matches!(lower.as_str(), "auto" | "default") {
        return Some(capitalize(&lower));
    }
    if let Some(rest) = lower.strip_prefix("gemini-") {
        let words: Vec<String> = rest
            .split('-')
            // Drop empty parts and date stamps.
            .filter(|w| !(w.is_empty() || w.len() >= 6 && w.bytes().all(|b| b.is_ascii_digit())))
            .map(capitalize)
            .collect();
        return Some(format!("Gemini {}", words.join(" ")).trim().to_string());
    }
    Some(id.chars().take(32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(id: &str) -> String {
        model_label(Some(id), None).unwrap()
    }

    #[test]
    fn claude_ids() {
        assert_eq!(l("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(l("claude-fable-5-2"), "Fable 5.2");
        assert_eq!(l("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(l("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(l("claude-opus-4-1-20250805[1m]"), "Opus 4.1");
        assert_eq!(l("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), "Sonnet 4.5");
        assert_eq!(l("claude-4.5-sonnet-thinking"), "Sonnet 4.5");
        assert_eq!(l("anthropic/claude-haiku-4-5"), "Haiku 4.5");
        assert_eq!(l("opus"), "Opus");
    }

    #[test]
    fn other_ids() {
        assert_eq!(l("gpt-5.5"), "gpt-5.5");
        assert_eq!(l("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(l("gemini-2.5-pro"), "Gemini 2.5 Pro");
        assert_eq!(l("opencode/big-pickle"), "big-pickle");
        assert_eq!(l("auto"), "Auto");
    }

    #[test]
    fn display_name_wins() {
        assert_eq!(model_label(Some("claude-opus-5-5"), Some("Opus 5.5 (1M context)")).as_deref(), Some("Opus 5.5"));
        assert_eq!(model_label(None, Some("  ")), None);
        assert_eq!(model_label(Some(""), None), None);
    }
}
