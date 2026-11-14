//! `agent.accounts`: is each agent CLI signed in, and what model does it start with.
//!
//! Everything comes from the CLIs' own local state: config files (parsed in memory, only
//! the fields named here are looked at), the presence of credential files, and `codex
//! login status`. No credential value is ever read out, logged or sent anywhere, and
//! nothing here talks to the network. Answers are cached for a minute because the
//! tracker asks whenever it opens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use muster_core::agents::AGENTS;
use muster_core::util::now_ms;
use muster_proto::AgentAccount;
use serde_json::Value;

const CACHE_FOR: Duration = Duration::from_secs(60);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// What one check found; the rest of [`AgentAccount`] is filled in around it.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    pub signed_in: Option<bool>,
    pub method: Option<String>,
    pub detail: Option<String>,
    pub default_model: Option<String>,
    pub usage_note: Option<String>,
}

fn json(text: Option<&str>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(t).ok())
}

fn non_empty(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// Claude Code: `oauthAccount` in `.claude.json` means a Claude.ai login (the token itself
/// is in the Keychain); `.credentials.json` is where it keeps the login without a Keychain.
pub fn claude(claude_json: Option<&str>, credentials_file: bool, api_key_env: bool, settings: Option<&str>) -> Found {
    let account = json(claude_json)
        .and_then(|v| v.get("oauthAccount").cloned())
        .is_some_and(|a| non_empty(a.get("accountUuid")).is_some());
    let (signed_in, method) = if account || credentials_file {
        (Some(true), Some("Claude account".to_string()))
    } else if api_key_env {
        (Some(true), Some("API key".to_string()))
    } else {
        (Some(false), None)
    };
    Found {
        signed_in,
        method,
        default_model: non_empty(json(settings).as_ref().and_then(|s| s.get("model")))
            .and_then(|m| muster_agents::model_label(Some(m), None)),
        usage_note: Some(
            "Session and weekly limits come from Claude Code's status line. Per-model weekly limits are only \
             available from Anthropic's usage endpoint, which Muster does not call."
                .into(),
        ),
        ..Found::default()
    }
}

/// `codex login status` prints "Logged in using ChatGPT", "Logged in using an API key - ..."
/// or "Not logged in". Only the method is taken from it, never the rest of the line.
pub fn codex(status: Option<&str>, config_toml: Option<&str>) -> Found {
    let (signed_in, method) = match status.map(str::trim) {
        Some(s) if s.starts_with("Logged in") => (
            Some(true),
            if s.contains("ChatGPT") {
                Some("ChatGPT".to_string())
            } else if s.contains("API key") {
                Some("API key".to_string())
            } else {
                None
            },
        ),
        Some(s) if s.starts_with("Not logged in") => (Some(false), None),
        _ => (None, None),
    };
    Found {
        signed_in,
        method,
        default_model: config_toml.and_then(|t| toml_top_string(t, "model")),
        usage_note: Some("Limits appear after a Codex turn in a Muster card reports them.".into()),
        ..Found::default()
    }
}

/// A top-level `key = "value"` from a TOML file (before the first table).
pub fn toml_top_string(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() == key {
            let v = v.trim();
            let v = v.strip_prefix('"')?;
            return v.split('"').next().map(str::to_string).filter(|s| !s.is_empty());
        }
    }
    None
}

/// cursor-agent keeps the signed-in user (not the token) in `cli-config.json`.
pub fn cursor(cli_config: Option<&str>) -> Found {
    let v = json(cli_config);
    let signed_in = v.as_ref().map(|v| {
        v.get("authInfo").is_some_and(|a| non_empty(a.get("userId")).is_some() || non_empty(a.get("authId")).is_some())
    });
    let model = v.as_ref().and_then(|v| v.get("model"));
    let default_model = model
        .and_then(|m| non_empty(m.get("displayNameShort")).or_else(|| non_empty(m.get("displayName"))))
        .map(str::to_string)
        .or_else(|| {
            model.and_then(|m| non_empty(m.get("modelId"))).and_then(|m| muster_agents::model_label(Some(m), None))
        });
    Found {
        signed_in,
        method: signed_in.filter(|s| *s).map(|_| "Cursor account".to_string()),
        default_model,
        usage_note: Some("Cursor reports plan usage only from its server.".into()),
        ..Found::default()
    }
}

/// Gemini CLI: `security.auth.selectedType` in its settings, and `oauth_creds.json` once a
/// Google login has happened.
pub fn gemini(settings: Option<&str>, oauth_creds: bool, api_key_env: bool) -> Found {
    let v = json(settings);
    let kind = v.as_ref().and_then(|v| non_empty(v.pointer("/security/auth/selectedType"))).map(str::to_string);
    let (signed_in, method) = match kind.as_deref() {
        Some("oauth-personal") | None if oauth_creds => (Some(true), Some("Google account".to_string())),
        Some("oauth-personal") => (Some(false), None),
        Some("gemini-api-key") => (Some(api_key_env).filter(|k| *k), Some("API key".to_string())),
        Some("vertex-ai") => (None, Some("Vertex AI".to_string())),
        _ => (None, None),
    };
    let model = v.as_ref().and_then(|v| non_empty(v.pointer("/model/name")).or_else(|| non_empty(v.get("model"))));
    Found {
        signed_in,
        method,
        default_model: model.and_then(|m| muster_agents::model_label(Some(m), None)),
        usage_note: Some("Gemini reports quota only from Google's server.".into()),
        ..Found::default()
    }
}

/// Antigravity CLI: its settings name the model; where it keeps the login is not known,
/// so sign-in is reported as unknown rather than guessed.
pub fn antigravity(settings: Option<&str>) -> Found {
    Found {
        default_model: non_empty(json(settings).as_ref().and_then(|v| v.get("model"))).map(str::to_string),
        usage_note: Some("Antigravity has no local usage record.".into()),
        ..Found::default()
    }
}

/// opencode: providers with stored credentials, and opencode accounts. It also works with
/// its free models without either.
pub fn opencode(providers: Option<Vec<String>>, accounts: u32) -> Found {
    let Some(providers) = providers else {
        return Found { usage_note: Some("opencode has no local usage record.".into()), ..Found::default() };
    };
    let signed_in = !providers.is_empty() || accounts > 0;
    Found {
        signed_in: Some(signed_in),
        method: (accounts > 0).then(|| "opencode account".to_string()),
        detail: if providers.is_empty() {
            (!signed_in).then(|| "free models only".to_string())
        } else {
            Some(providers.join(", "))
        },
        usage_note: Some("opencode has no local usage record.".into()),
        ..Found::default()
    }
}

/// Provider ids with stored credentials and the number of opencode accounts, from
/// opencode's own database. Only those columns are selected; tokens are never read.
fn opencode_db(path: &Path) -> Option<(Vec<String>, u32)> {
    use rusqlite::{Connection, OpenFlags};
    if !path.is_file() {
        return None;
    }
    let db =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    db.busy_timeout(Duration::from_millis(200)).ok()?;
    let mut st = db.prepare("SELECT DISTINCT integration_id FROM credential WHERE integration_id IS NOT NULL").ok()?;
    let providers = st.query_map([], |r| r.get::<_, String>(0)).ok()?.filter_map(Result::ok).collect();
    let accounts = db.query_row("SELECT COUNT(*) FROM account", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    Some((providers, accounts.max(0) as u32))
}

fn read(p: &Path) -> Option<String> {
    // Config files are small; anything huge is not one.
    if std::fs::metadata(p).ok()?.len() > 8 << 20 {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

fn env_dir(var: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or(default)
}

fn env_set(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty())
}

async fn command_output(program: &Path, args: &[&str]) -> Option<String> {
    let run =
        tokio::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true).output();
    let out = tokio::time::timeout(COMMAND_TIMEOUT, run).await.ok()?.ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(text)
}

/// Checks every agent. `installed` maps a CLI's command name to its full path.
pub async fn check(installed: &HashMap<String, PathBuf>) -> Vec<AgentAccount> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let codex_home = env_dir("CODEX_HOME", home.join(".codex"));
    let codex_status = match installed.get("codex") {
        Some(p) => command_output(p, &["login", "status"]).await,
        None => None,
    };
    let found = tokio::task::spawn_blocking(move || {
        let mut found: HashMap<&'static str, Found> = HashMap::new();
        let claude_dir = env_dir("CLAUDE_CONFIG_DIR", home.join(".claude"));
        let claude_json = if std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
            claude_dir.join(".claude.json")
        } else {
            home.join(".claude.json")
        };
        found.insert(
            "claude",
            claude(
                read(&claude_json).as_deref(),
                claude_dir.join(".credentials.json").is_file(),
                env_set("ANTHROPIC_API_KEY"),
                read(&claude_dir.join("settings.json")).as_deref(),
            ),
        );
        found.insert("codex", codex(codex_status.as_deref(), read(&codex_home.join("config.toml")).as_deref()));
        found.insert("cursor", cursor(read(&home.join(".cursor/cli-config.json")).as_deref()));
        let gemini_dir = home.join(".gemini");
        found.insert(
            "gemini",
            gemini(
                read(&gemini_dir.join("settings.json")).as_deref(),
                gemini_dir.join("oauth_creds.json").is_file(),
                env_set("GEMINI_API_KEY"),
            ),
        );
        found.insert("antigravity", antigravity(read(&gemini_dir.join("antigravity-cli/settings.json")).as_deref()));
        let data = env_dir("XDG_DATA_HOME", home.join(".local/share"));
        let oc = opencode_db(&data.join("opencode/opencode.db"));
        found.insert("opencode", opencode(oc.as_ref().map(|o| o.0.clone()), oc.map_or(0, |o| o.1)));
        found
    })
    .await
    .unwrap_or_default();
    let mut found = found;
    let now = now_ms();
    AGENTS
        .iter()
        .filter_map(|a| a.command.map(|cmd| (a, cmd)))
        .map(|(a, cmd)| {
            let installed = installed.contains_key(cmd);
            let f = found.remove(a.id).unwrap_or_default();
            AgentAccount {
                agent_id: a.id.to_string(),
                installed,
                signed_in: f.signed_in,
                method: f.method,
                detail: f.detail,
                default_model: f.default_model,
                usage_note: f.usage_note,
                checked_at: now,
            }
        })
        .collect()
}

#[derive(Default)]
pub struct Cache(Mutex<Option<(Instant, Vec<AgentAccount>)>>);

impl Cache {
    pub fn get(&self, refresh: bool) -> Option<Vec<AgentAccount>> {
        let c = self.0.lock().unwrap();
        c.as_ref().filter(|(at, _)| !refresh && at.elapsed() < CACHE_FOR).map(|(_, v)| v.clone())
    }

    pub fn put(&self, v: Vec<AgentAccount>) {
        *self.0.lock().unwrap() = Some((Instant::now(), v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_sign_in() {
        let j = r#"{"oauthAccount":{"accountUuid":"u-1","emailAddress":"x@example.com"}}"#;
        let f = claude(Some(j), false, false, Some(r#"{"model":"opus"}"#));
        assert_eq!(f.signed_in, Some(true));
        assert_eq!(f.method.as_deref(), Some("Claude account"));
        assert_eq!(f.default_model.as_deref(), Some("Opus"));
        assert_eq!(claude(Some("{}"), false, true, None).method.as_deref(), Some("API key"));
        assert_eq!(claude(None, false, false, None).signed_in, Some(false));
        assert_eq!(claude(None, true, false, None).signed_in, Some(true));
    }

    #[test]
    fn codex_status_lines() {
        let f = codex(Some("Logged in using ChatGPT\n"), Some("model = \"gpt-5.5\"\n[x]\nmodel = \"no\""));
        assert_eq!(
            (f.signed_in, f.method.as_deref(), f.default_model.as_deref()),
            (Some(true), Some("ChatGPT"), Some("gpt-5.5"))
        );
        let f = codex(Some("Logged in using an API key - sk-proj-***ABCD"), None);
        assert_eq!(f.method.as_deref(), Some("API key"));
        assert_eq!(codex(Some("Not logged in"), None).signed_in, Some(false));
        assert_eq!(codex(None, None).signed_in, None);
        assert_eq!(toml_top_string("[a]\nmodel = \"x\"", "model"), None);
    }

    #[test]
    fn cursor_config() {
        let j = r#"{"authInfo":{"userId":"1","email":"x@example.com"},"model":{"modelId":"gpt-5","displayNameShort":"GPT-5"}}"#;
        let f = cursor(Some(j));
        assert_eq!((f.signed_in, f.default_model.as_deref()), (Some(true), Some("GPT-5")));
        assert_eq!(cursor(Some("{}")).signed_in, Some(false));
        assert_eq!(cursor(None).signed_in, None);
    }

    #[test]
    fn gemini_settings() {
        let s = r#"{"security":{"auth":{"selectedType":"oauth-personal"}},"model":{"name":"gemini-2.5-pro"}}"#;
        let f = gemini(Some(s), true, false);
        assert_eq!((f.signed_in, f.method.as_deref()), (Some(true), Some("Google account")));
        assert_eq!(f.default_model.as_deref(), Some("Gemini 2.5 Pro"));
        assert_eq!(gemini(Some(s), false, false).signed_in, Some(false));
        assert_eq!(gemini(None, true, false).signed_in, Some(true));
        assert_eq!(gemini(None, false, false).signed_in, None);
        let k = r#"{"security":{"auth":{"selectedType":"gemini-api-key"}}}"#;
        assert_eq!(gemini(Some(k), false, true).signed_in, Some(true));
    }

    #[test]
    fn opencode_and_antigravity() {
        let f = opencode(Some(vec![]), 0);
        assert_eq!((f.signed_in, f.detail.as_deref()), (Some(false), Some("free models only")));
        let f = opencode(Some(vec!["anthropic".into(), "openai".into()]), 0);
        assert_eq!((f.signed_in, f.detail.as_deref()), (Some(true), Some("anthropic, openai")));
        assert_eq!(opencode(None, 0).signed_in, None);
        let a = antigravity(Some(r#"{"model":"Gemini 3.1 Pro (Low)"}"#));
        assert_eq!((a.signed_in, a.default_model.as_deref()), (None, Some("Gemini 3.1 Pro (Low)")));
    }
}
