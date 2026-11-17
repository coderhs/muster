# Agent integrations

What Muster relies on for each agent CLI, and how each fact was established.
`[run]` = verified by running the CLI on this machine; `[docs]` = from the CLI's
documentation or `--help` only, not exercised.

Code: `crates/muster-agents` (adapters, receiver, status tracker, transcript and usage
readers) and `crates/muster-hook` (the forwarder binary). The crate docs in
`crates/muster-agents/src/lib.rs` show the calls the daemon makes.

## Support matrix

| | Claude Code | Codex | Cursor | opencode | Gemini CLI | Antigravity |
|---|---|---|---|---|---|---|
| Version checked | 2.1.323 | 0.139.0 | 2026.11.05 | 2.0.32 | 0.46.0 | 1.2.2 |
| Could run a turn here | yes | no (not logged in) | yes | no (free quota used up) | no (account tier refused) | not tried |
| Launch with prompt | [run] | built from help | [run] | `--prompt` from help; `run` [run] | from help | from help |
| Muster picks the session id | [run] `--session-id` | no; found from rollout [run] | possible (`create-chat`) [run], not used | [run] `--session` | `--session-id` from help | no |
| Resume | [run] | `codex resume <id>` from help | [run] `--resume <id>` | `--session <id>` from help | `--resume` from help | `--conversation` from help |
| Status source | hooks via `--settings` [run] | rollout log [run] | hooks via `--plugin-dir` [run] | CLI API polling [run] | terminal | terminal |
| needs_you (approvals/questions) | [run] | not visible | not visible | not wired | terminal | terminal |
| Answer approvals from the board | [run] incl. questions, plans | no | no (see Cursor) | documented, not wired | no | no |
| Per-launch instructions | [run] | [run] (prompt input) | [run] (manual plugin) | no flag | no | no |
| Chat transcript | [run] | [run] (log format) | [run] (text only) | [run] (API JSON) | no | no |
| Context usage | [run] | [run] (log format) | [run] tokens, no window | tokens only | no | no |
| Usage limits | [run] status line | rollout `rate_limits` (log format) | none found | none | no | no |

`[run]` = verified by running the CLI on this machine; `[docs]` = documentation only.
"Log format" = parsed from real files on disk without a fresh turn.

## Claude Code (2.1.323)

### Verification runs

Six `claude` processes, ten model turns, all prompts trivial ("reply with the single
word ok", "touch <file>"). Interactive runs were driven in tmux on a private socket
(`-L muster-test-<n>`), then cleaned up. Payloads captured in runs 2 and 4 are the test
fixtures in `crates/muster-agents/tests/fixtures/claude/` (user name, account ids, hook
outputs of other tools and thinking text removed).

| # | Mode | Checked |
|---|---|---|
| 1 | `-p` | nothing captured (socket path too long, see below) |
| 2 | `-p --session-id --settings` | hooks from `--settings` fire; chosen session id honoured; event order |
| 3 | interactive, untrusted folder | trust dialog appears first; no hook fires before it; declined, no API call |
| 4 | interactive, trusted folder | event order, status line feed with rate limits, permission allowed through the hook, permission rejected in the terminal, `/clear`, `/exit` |
| 5 | interactive `--resume` | resume keeps the session id; `SessionStart` `source: resume` |
| 6 | interactive, launched from the library's own command and settings file, events through `HookReceiver` | allow and deny answered through the receiver, an answer typed in the terminal (`1`), release of the left-over hook, `--` before the prompt |

Runs 2 to 5 inherited the environment of the session that drove them; run 6 used a
clean environment (`env -i` with HOME, PATH, TERM, LANG), as the daemon will.

### Launch and resume

- New session: `claude --session-id <uuid> --settings <file> [extra args] -- <prompt>`.
  The id must be a UUID; Muster generates a v4 one. `[run]` The prompt after `--` is
  submitted as the first message `[run]`.
- Resume: `claude --resume <session-id> --settings <file>`. Same id afterwards, no new
  file. `[run]` `--fork-session` would create a new id `[docs]` (not used).
- In a folder the user has never trusted, the interactive CLI first shows "Is this a
  project you created or one you trust?" (default "No, exit"). No hook, not even
  `SessionStart`, runs until it is accepted. `[run]` The daemon cannot learn of this from
  hooks; a card that produces no `SessionStart` within a few seconds of launch should be
  shown as needing the user (the terminal shows the dialog). `-p` skips the dialog `[docs]`.

### Hooks: per launch, no config edits

- `--settings <path-or-json>` loads an extra settings layer for that process only.
  Hooks in it fire alongside the user's own hooks from `~/.claude/settings.json` and
  project files. `[run]` (The user's own hooks kept running in every test run.)
- Muster writes its settings file under its own state dir
  (`<state_dir>/claude/settings-<hash>.json`); nothing under `~/.claude` is written.
- Hook entries use the exec form `{"type":"command","command":"<muster-hook>","args":["hook","<Event>"],"timeout":10}`,
  so no shell is spawned per event. `[run]`
- `timeout` is in seconds; default 600 for command hooks `[docs]`. Muster uses 10 for
  forwards and `approval_wait + 30` for `PermissionRequest`.
- A hook exiting 0 with no output changes nothing `[run]`. Exit codes other than 0 and 2
  are reported to the user as hook errors `[docs]`; `muster-hook` always exits 0.
- `disableAllHooks` (in any settings layer) turns off hooks and the status line,
  including Muster's `[docs]`. `--bare` and `--safe-mode` also skip hooks `[docs]`; Muster
  must not pass them.
- macOS limits Unix socket paths to 104 bytes; a socket in a deep temp directory failed
  to bind `[run]`. `$MUSTER_HOME/hook.sock` (~60 bytes) is fine.
- `muster-hook` with the daemon down returns in about 5 ms, no output, exit 0 `[run]`
  (the first exec of a freshly built binary took ~0.4 s once, macOS first-launch check).

### Events

Common payload fields: `session_id`, `transcript_path`, `cwd`, `hook_event_name`;
most also `permission_mode`, `prompt_id`, `effort`; interactive runs add
`scratchpad_dir`. `agent_id` / `agent_type` appear inside subagents. `[run]`

| Event | Fields used | Muster event | Status effect |
|---|---|---|---|
| `SessionStart` | `source` (`startup`, `resume`, `clear` seen; `compact`, `fork` documented), `model` (string) | `sessionStarted` | idle (not on compact) |
| `UserPromptSubmit` | `prompt` | `turnStarted` | running, sets turn start |
| `PreToolUse` | `tool_name`, `tool_input`, `tool_use_id` | `toolStarted` (`questionAsked` for `AskUserQuestion` `[docs]`) | activity |
| `PermissionRequest` | `tool_name`, `tool_input`, `permission_suggestions` (no `tool_use_id`) | `permissionRequested` | needs_you |
| `PostToolUse` | `tool_name`, `tool_input`, `tool_response`, `tool_use_id`, `duration_ms` | `toolFinished` | running if it was waiting |
| `PostToolUseFailure` | `error`, `is_interrupt` `[docs]` | `toolFinished ok:false` | as above |
| `Stop` | `last_assistant_message`, `background_tasks` | `turnFinished` | done |
| `StopFailure` | `error` (`rate_limit`, `authentication_failed`, ...) `[docs]` | `turnFinished error` | done |
| `SessionEnd` | `reason`: `other` (end of `-p`), `clear`, `prompt_input_exit` (`/exit`) seen; `logout`, `resume` documented | `sessionEnded` (`exited` false for `clear`/`resume`) | exited |
| `SubagentStart` / `SubagentStop` | `agent_id`, `agent_type`, `agent_transcript_path` | `subagentStarted/Stopped` | none |
| `Notification` | `notification_type` (`permission_prompt`, `idle_prompt`, `elicitation_dialog`, ...) `[docs]` | `notification` | needs_you for prompt/elicitation types |
| `PreCompact` / `PostCompact` | `trigger` `[docs]` | `compactionStarted/Finished` | activity |
| `PermissionDenied` | auto mode refusals `[docs]` | `permissionAutoDenied` | none |
| `Elicitation` | MCP input request `[docs]` | `inputRequested` | needs_you |
| `InstructionsLoaded`, `PostToolBatch`, others | | `other` | activity |

Observed sequences `[run]`:

- `-p` turn: `InstructionsLoaded, SessionStart(startup), UserPromptSubmit, Stop, SessionEnd(other)`.
- Interactive with a prompt: `SessionStart(startup)`, status line, `InstructionsLoaded`,
  `UserPromptSubmit`, `Stop`, status line.
- Tool turn: `UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PostToolBatch, Stop`.
- After most turns Claude Code runs an internal helper agent (its next-prompt
  suggestion) and reports `SubagentStop` with an empty `agent_type` and no
  `SubagentStart`. Muster ignores it for status.
- `/clear`: `SessionEnd(clear)` for the old id, then `SessionStart(clear)` with a **new
  session id** and transcript path. The tracker reports `SessionId` changes; the daemon
  must store the new id. After `/clear` without a new prompt the new id has only a small
  transcript; resuming the older id still works.
- Escape on a permission prompt rejects the call and **interrupts the turn: no `Stop`,
  no `PostToolUseFailure`** follows. The transcript gets a `tool_result` with
  `is_error: true` ("The user doesn't want to proceed...") and a user text
  `[Request interrupted by user for tool use]`, then a `system` line
  `subtype: turn_duration`. `AgentAdapter::turn_state` detects this; the daemon calls it
  after a `PermissionResolved { by: elsewhere }` (and when a `running` card's terminal goes
  quiet) and applies it with `StatusTracker::apply_turn_state`.
- The `permission_prompt` Notification was not observed within the 6-second hold in run
  4 (not investigated further; Muster does not depend on it).

### Answering permission requests from the board

- A `PermissionRequest` hook can print
  `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`
  to allow; the terminal shows "Allowed by PermissionRequest hook". `[run]`
- Deny: `"decision":{"behavior":"deny","message":"..."}`; the terminal shows "Denied by
  PermissionRequest hook" with the reason, Claude receives the message and the turn
  continues (it replied "refused"). `[run]` `"interrupt": true` also stops the turn `[docs]`.
- "Allow and remember" sends back `updatedPermissions` built from the request's
  `permission_suggestions`, keeping only `addRules` and `addDirectories` entries (the
  suggestions also include `setMode` to acceptEdits, which Muster never applies
  silently). Format `[docs]`, not exercised.
- **Claude Code shows its own prompt at the same time as the hook runs.** `[run]` So the
  terminal is never blocked by the hold, and "timeout, then the CLI prompt takes over" is
  automatic: the prompt is already there. Whichever answers first wins.
- If the user rejects in the terminal (Escape), Claude Code kills the hook process (the
  receiver sees EOF). `[run]` If the user **allows** in the terminal (`1`), the hook keeps
  running until its wait ends. `[run]` The receiver therefore releases a waiting hook when
  the same card reports `PostToolUse`/`PostToolUseFailure` for the same tool and input, or
  `Stop`, `StopFailure`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`; it then
  emits `PermissionResolved { by: elsewhere }`. Verified in run 6: released right after
  `PostToolUse`, no hook process left. When the wait runs out first, the hook prints
  nothing and exits; the receiver emits the same event (verified at the 120 s wait).
- Default wait is 30 minutes (`HookConfig::approval_wait`); the CLI's own cancel is at
  wait + 30 s.
- Other tools' `PermissionRequest` hooks in the user's settings run in parallel with
  Muster's. How Claude Code combines conflicting decisions from several hooks was not
  tested (the docs give deny > ask > allow for `PreToolUse`).
- Terminal fallback (`AgentAdapter::permission_keys`): allow once = `1` `[run]` (the
  menu starts with "1. Yes", cursor on it); deny = `Escape` `[run]` (rejects and
  interrupts the turn). The "No" entry's number varies (3 or 4 options depending on the
  offered "Yes, and ..." variants) and "remember" variants differ per tool, so there is
  no key for "remember" or for "deny without interrupting".

### Transcript

- Location: `transcript_path` in every hook payload `[run]`; otherwise
  `$CLAUDE_CONFIG_DIR` or `~/.claude`, then `projects/<cwd with every non-alphanumeric
  character replaced by '-'>/<session-id>.jsonl` `[run]` (two directory names checked),
  with a scan of `projects/*/` as fallback. Transcripts are deleted after
  `cleanupPeriodDays` (default 30) `[docs]`.
- JSONL with many line types besides the conversation: `attachment`,
  `api-request*`, `file-history-*`, `cost-state`, `ai-title`, `last-prompt`, `mode`,
  `permission-mode`, `session-owner`, ... Lines of 100+ KB are normal; a few short turns
  made a 510 KB file `[run]`. The reader goes backwards in 64 KB chunks, parses only lines
  containing `"type":"user"` / `"type":"assistant"`, and stops after 32 MB per call.
- Assistant replies are written one line per content block, sharing `message.id`;
  they are merged into one chat message (id = `message.id`). Thinking blocks are reduced
  to a `thinking` marker. Tool calls become one-line summaries, tool results their first
  line (200 chars). Lines with `isSidechain` or `isMeta`, synthetic assistant lines
  (`model: "<synthetic>"`, e.g. "No response requested." written on resume after an
  interrupt) and local command echoes are skipped. `[run]`
- Context usage from the transcript: `input_tokens + cache_creation_input_tokens +
  cache_read_input_tokens` of the newest main-thread assistant line `[run]`. The window
  size is not in the transcript (the status line said 1,000,000 for `claude-fable-5-2`
  on this account), so the transcript fallback reports the window only for `[1m]`
  model ids.

### Usage limits

Investigated in order of invasiveness:

1. **Status line input (chosen).** `[run]` A `statusLine` passed in `--settings` runs for
   that session. Its stdin JSON carries `context_window` (`context_window_size`,
   `current_usage`, `used_percentage`) on every refresh and, once the session has had an
   API response, `rate_limits.five_hour` / `rate_limits.seven_day` with `used_percentage`
   and `resets_at` (unix seconds), observed as 25 % / 12 % and later 33 % / 13 %. The
   first refresh, before any response, has `rate_limits` absent and `current_usage`
   null. Also `model`, `cost.total_cost_usd`, `session_name`, `version`. A
   `spend_limit` window exists for gateway setups `[docs]`. Refreshes happen after each
   assistant message, on permission mode changes and at `resets_at` `[docs]`; the feed
   goes quiet while the session is idle, and only sessions with a response report limits.
   Limits are account-wide, but each process reports the `anthropic-ratelimit-unified-5h-*`
   / `-7d-*` headers of its own latest response (checked in the 2.1.325 bundle), and
   leaves a window out once that copy's `resets_at` has passed. So the newest snapshot is
   not the current one: an idle session's refresh repeats old numbers. The daemon merges
   reports per window (`crates/musterd/src/usage.rs`): larger `used_percentage` within the
   same `resets_at`, later `resets_at` replaces, missing windows are kept. Claude Code's
   own `/usage` screen also lists per-model weekly windows (`seven_day_sonnet`,
   `model_scoped[]`, e.g. "Current week (Fable)"); those come only from
   `GET api.anthropic.com/api/oauth/usage` with the user's OAuth token, not from response
   headers, so Muster does not have them.
   - The `--settings` status line replaces the user's own for that session (one value
     wins `[docs]`). To keep the user's status line, the adapter reads (read-only) the
     effective `statusLine` from `<cwd>/.claude/settings.local.json`,
     `<cwd>/.claude/settings.json`, then `~/.claude/settings.json`, passes its command in
     `MUSTER_STATUSLINE_CHAIN`, and `muster-hook statusline` runs it with the same stdin
     and prints its output. `padding`, `refreshInterval` and `hideVimModeIndicator` are
     copied. The chaining is covered by a unit test of the binary; with the real CLI
     (run 6) only the forwarding was checked, not that the user's own status line text
     appeared on screen (theirs prints nothing outside its own app). Set
     `HookConfig::capture_statusline = false` to leave the status line alone (no rate
     limits then).
2. **Hooks.** No hook carries limit numbers. `StopFailure` reports `error: rate_limit`
   and Notification types `quota_auto_resume_*` exist `[docs]`; both only say a limit was
   hit, not how much is left. Not used for the strip.
3. **The CLI's own endpoint (not implemented).** The binary contains
   `/api/oauth/usage` and reads `anthropic-ratelimit-unified-5h-*` / `-7d-*` response
   headers (strings in the executable). Using it would mean reading the OAuth token from
   the macOS Keychain entry Claude Code owns. Not done: it is undocumented, and the status
   line already gives the same numbers whenever any session is active. If the strip must
   show limits with no active session, this is the remaining option and must stay
   read-only (never refresh or store the token).

### Round 2 additions (Claude Code)

Six more `claude` processes (four used the model, ten turns in all; one failed before any request because of a bad model name).

- **Questions (AskUserQuestion) `[run]`**: Claude Code sends a `PermissionRequest` with
  `tool_name: "AskUserQuestion"` and the questions in `tool_input.questions`
  (`question`, `header`, `options[{label, description}]`, `multiSelect`). The terminal
  shows its own question picker at the same time. The answer is
  `decision: {"behavior":"allow","updatedInput":{"questions":<original>,"answers":{"<question text>":"<label>"}}}`.
  Verified: "User answered Claude's questions: … → blue" and the model replied "blue".
  The library uses `PromptKind::Question` with typed questions and
  `PermissionDecision::Answer`. The `PreToolUse` for the same call also produces
  `questionAsked`. Several labels are joined with ", " `[docs]`.
- **Plan approval (ExitPlanMode) `[run]`**: this is a distinct `PermissionRequest`
  (`tool_name: "ExitPlanMode"`, `tool_input.plan`, `tool_input.planFilePath`). The answer
  `{"behavior":"allow","updatedInput":<original input>}` approved the plan, and Claude went on
  to write the file (that Write asked separately and was allowed through the hook).
  `ApprovePlan { accept_edits: true }` adds a `setMode` acceptEdits grant `[docs]`. Rejecting
  (`Deny` with a message) is `[docs]`. The terminal offered "1. Yes, and use auto mode / 2. Yes,
  manually approve edits / 3. Tell Claude what to change". Typing `3`, the feedback text and
  Enter sent feedback, and Claude revised the plan `[run]`. No key fallback is offered for
  plans or questions because the menus vary. In plan mode Claude itself writes the plan
  file under `~/.claude/plans/`.
- A `permission_prompt` Notification arrived a few seconds after the plan prompt
  appeared `[run]`.
- **Subagent `[run]`**: the Agent tool now runs subagents in the background:
  `PreToolUse(Agent)`, then `SubagentStart{agent_type:"general-purpose"}`, then `PostToolUse`.
  `Stop` reports `background_tasks:[{status:"running",...}]` (exposed as
  `TurnFinished.background_tasks`). Later `SubagentStop` arrives with
  `agent_transcript_path` (`<session dir>/subagents/agent-<id>.jsonl`), followed by a
  new `UserPromptSubmit` whose prompt is a `<task-notification>` block, and a second `Stop`.
- **Compaction `[run]`** (`/compact`): `PreCompact{trigger:"manual"}`, a SubagentStop from
  the internal summariser, `SessionStart{source:"compact"}` with the **same** session
  id, then `PostCompact{trigger, compact_summary}`. No `Stop` follows, so compaction
  no longer changes the status.
- **StopFailure `[run]`**: `--model claude-nonexistent-9` gives
  `StopFailure{error:"model_not_found", last_assistant_message}`.
- **Per-launch instructions `[run]`**: `--append-system-prompt-file <file>` (Claude quoted a
  codeword from it) and `--plugin-dir <dir>` with `<dir>/.claude-plugin/plugin.json` and
  `<dir>/skills/muster/SKILL.md` (the skill is listed as `muster:muster` and read on
  demand). Neither writes to `~/.claude` or the repo. Skills used to come through
  `--add-dir <dir>/.claude/skills/...`, but from 2.1.x Claude Code asks the user to trust
  every `--add-dir` directory on each launch and resume; `--plugin-dir` asks nothing
  `[run]`. The prompt still goes after `--`. The system prompt is recorded on the first
  request and reused on resume (`--help`). `LaunchSpec::instructions` / `plugin_dir`.
- **Other tools' PermissionRequest hooks `[run]`**: with an extra hook that denied
  immediately and Muster allowing after 2 s, the call was denied. With the extra hook
  allowing immediately and Muster set to deny after 2 s, the call ran within 0.5 s without
  waiting for Muster. Muster's waiting hook was then released by the following
  `PostToolUse`. So any other hook that answers wins over the board (an immediate allow
  wins at once, a deny wins outright). The board cannot override it. If the user has such
  hooks, the board's answers are advisory. Muster cannot detect them without reading the
  user's settings. `muster_agents::claude::user_statusline_command` shows that approach
  for the status line; for hooks it is not implemented.

### Not verified

- `Elicitation` (MCP input) and auto compaction: mapped from the docs, not exercised.
- `updatedPermissions` ("remember") round trip.
- Behaviour when `--settings` hooks meet `disableAllHooks` or managed
  `allowManagedHooksOnly` (documented to disable them).

## Codex (codex-cli 0.139.0)

Not logged in on this machine (`codex login status`: "Not logged in"), so no model turn
was possible. I made no login attempt. Six `codex exec` runs went unauthenticated; five of
them used a throwaway `CODEX_HOME`. Codex still writes its rollout log and fails at the
API with 401. The first run used the real home and left one rollout file in
`~/.codex/sessions/2026/11/14/`; `config.toml` was not changed. One run of
`codex debug prompt-input` was also made; it needs no model.

- **Launch**: `codex [-c key=value...] [extra] -- <prompt>`. **Resume**: `codex resume
  [options] <session-id> [prompt]` (from `--help`). Codex cannot take a session id.
- **Session discovery `[run]`**: every session writes
  `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<local time>-<uuid>.jsonl`. Its first line is
  `session_meta{id, cwd, timestamp, cli_version, source}`. `exec` also prints
  "session id: <uuid>". `find_session(cwd, started_after)` matches by cwd and creation
  time.
- **Status from the rollout log `[run]`**: `event_msg` lines `task_started`,
  `user_message`, `task_complete{last_agent_message}` (seen in a fresh run);
  `token_count`, `agent_message` and `turn_aborted{reason}` (seen in the user's earlier
  rollouts); `response_item` lines `message` (roles developer/user/assistant),
  `reasoning`, `function_call` / `function_call_output`, `custom_tool_call(_output)`,
  `web_search_call`. `log_events` maps these to turn/tool/usage events. Approval prompts
  do not appear in the log (none of the user's rollouts has one), so Codex cards never show
  `needs_you`. Neither the event order of an approval nor the TUI's log behaviour could
  be verified without a login.
- **Context and limits (log format, verified on real files)**: `token_count.info` has
  `last_token_usage{input_tokens, cached_input_tokens, output_tokens, ...}` and
  `model_context_window` (258400). `rate_limits` has `primary` / `secondary`
  `{used_percent, window_minutes, resets_at}` plus `plan_type`. The user's file showed a
  43200-minute (30-day) primary window. Each turn starts with a `turn_context` line whose
  `model` (e.g. `gpt-5.5`) is shown on the card. Context used = last input + output tokens
  (cached tokens are part of input).
- **Instructions `[run]`**: `-c developer_instructions="..."` appears as a developer
  message in `codex debug prompt-input` (no model call).
- **Hooks (not used; user decision)**: Codex has Claude-like hooks (`hooks.json` or
  `[hooks]` in config, events SessionStart, UserPromptSubmit, PreToolUse,
  PermissionRequest with allow/deny output, PostToolUse, Stop, Interrupt, ... `[docs]`).
  Every non-managed hook must be trusted by hash in `/hooks`, and Codex stores the trust in
  the user's `config.toml` (`[hooks.state."<file>:<event>:i:j"] trusted_hash`).
  Untrusted hooks are skipped. `--dangerously-bypass-hook-trust` runs them without trust
  for one invocation, but it also turns off the trust check for every other hook.
  In testing, hooks supplied with `-c hooks.<Event>=[...]` and with `hooks.json` in a temp
  `CODEX_HOME`, with and without the bypass flag, **did not fire at all** in unauthenticated
  `exec` runs (likely because of the missing login; not established).
  `codex::inline_hook_args` builds the `-c` form if it is ever enabled. **Decision for the
  user**: keep log-only status (no approvals from the board), or allow Muster to pass
  `--dangerously-bypass-hook-trust` with its own per-launch hooks.

## Cursor Agent (cursor-agent 2026.11.05)

Logged in. Four model runs (two `-p`, two interactive) plus two `create-chat`
calls (local, no model).

- **Hooks per launch `[run]`**: `--plugin-dir <dir>`, where the dir holds
  `.cursor-plugin/plugin.json` (`{"name": ...}`) and `hooks/hooks.json`
  (`{"version":1,"hooks":{"<event>":[{"command":...,"timeout":10}]}}`), loads hooks
  for that run only. `~/.cursor/hooks.json` was not touched. Interactive runs fired
  `beforeSubmitPrompt, sessionStart, afterAgentThought, preToolUse, beforeShellExecution,
  afterShellExecution, postToolUse, stop{status:"completed", input_tokens, output_tokens,
  cache_*}, afterAgentResponse, sessionEnd`. `-p` runs fired only `sessionStart`,
  `afterAgentThought`, `sessionEnd` (and on `--resume` only `sessionEnd`). Payloads carry
  `conversation_id` (= session id), `generation_id`, `model`, `workspace_roots`,
  `transcript_path` (null early), `user_email` (dropped from fixtures). `beforeSubmitPrompt`
  arrives before `sessionStart` on the first turn.
- **Instructions `[run]`**: plugin `rules/*.mdc` were not applied in a `-p` run. A
  `sessionStart` hook printing `{"additional_context": "..."}` worked interactively (the
  agent quoted the codeword). It does not fire when starting a `create-chat` chat with
  `--resume`, which is why the adapter lets Cursor choose the id and reads it from the
  first event. Launched through the library with a preset id, the instructions were indeed
  missing `[run]`.
- **Session id**: `create-chat` prints a UUID, and `--resume <id>` uses it `[run]`.
- **Transcript `[run]`**: `~/.cursor/projects/<workspace path, '/'→'-'>/agent-transcripts/<id>/<id>.jsonl`,
  where the workspace is the git root in interactive runs. Lines are `{role, message.content[{type:"text"}]}`
  with the user text wrapped in `<timestamp>` / `<user_query>`, and
  `{"type":"turn_ended","status":"success"}`. Tool calls are not in it. Thinking text
  was merged into the assistant text in one `-p` transcript and cannot be separated.
- **Approvals**: the user's config runs in "Run Everything" mode, so no prompt appeared.
  Hooks can only gate a call before Cursor's own prompt (`beforeShellExecution` /
  `preToolUse` `permission`) `[docs]`. Holding them for the board would make every command
  wait, so it is not enabled, and Cursor cards do not show `needs_you`.
- **Usage**: `stop` gives token counts and `preCompact` gives `context_window_size`
  `[docs]`. No rate-limit source was found (`about --format json` has none).

## opencode (v2.0.32)

No provider logged in. The free models answered "Free usage limit reached" (429), so no
turn completed. Two runs: `opencode run` once, plus read-only `opencode api` queries and
`opencode models`.

- Sessions live in a shared background service; `opencode service status` printed its
  URL. Direct HTTP gets 401. The service password is in `~/.local/state/opencode/service.json`
  and Muster does not read it. `opencode api GET <path>` authenticates by itself `[run]`.
- **Preset session id `[run]`**: `opencode run --session ses_muster_test_0001 ...`
  created a session with exactly that id. TUI launch is `opencode --session <id> --prompt
  <p>` (flags from `--help`).
- **Status `[run]` (API shape)**: `GET /api/session/<id>` →
  `{data:{id, time{created, updated, idle}, outcome, tokens{input, output, reasoning,
  cache{read, write}}, cost, location}}`. `GET /api/session/<id>/message` lists
  `idle` / `assistant` / `user` records, newest first. `GET /api/event` through the CLI
  printed nothing within 6 s, so the event stream is not used.
- Permissions (`GET/POST /api/session/<id>/permission[/<rid>/reply]`) and plugins `[docs]`
  only, not wired.

## Gemini CLI (0.46.0) and Antigravity (`agy` 1.2.2)

- The Antigravity CLI's command is `agy` (the agent table used to look for
  `antigravity`, so it showed as not installed).

- Gemini: authentication fails ("IneligibleTierError: This client is no longer supported
  for Gemini Code Assist for individuals"), so no runs. Flags from `--help`:
  `--session-id <uuid>`, `--resume <latest|index>`, `-i <prompt>`. Resume by id is
  unverified.
- Antigravity: flags from `--help`: `-i <prompt>`, `--conversation <id>`, `--continue`,
  `--add-dir`. No way to preset an id; no runs.
- Both use terminal-activity status (`StatusSource::Terminal`).

## Hook authenticity (all hooked agents)

Every card process has `MUSTER_HOOK_SOCK` in its environment, and an envelope names its
card in free text. Without a check, an agent in one card could send events as another card.
Two defences are in `muster-agents` / `muster-hook`; both are covered by unit tests.

- **Per-launch hook secret.** The daemon makes a secret per launch
  (`muster_agents::new_hook_secret()`, 32 random bytes as hex), sets
  `LaunchSpec::hook_secret`, and registers it with `HookReceiver::set_card_secret(card, s)`.
  The Claude and Cursor adapters pass it to the session only through the environment
  (`MUSTER_HOOK_SECRET`). The generated settings and plugin files stay shared and contain
  no secret. `muster-hook` copies it into the envelope (`"secret"`). The receiver compares
  it in constant time and drops envelopes with a missing or wrong secret, and those for
  unregistered cards. A dropped envelope that waits for a reply gets an empty line, so the
  agent's own prompt takes over. Drops are counted (`HookReceiver::dropped_events()`). The
  secret is removed before an envelope reaches the raw tap or an adapter.
  `set_require_secret(false)` accepts unregistered cards, for migration only.
  Limits: this stops another card's agent from sending events in a card's name through
  the documented path. It does not stop a determined process running as the same user,
  which can read Muster's state and may be able to read other processes' environments.
- **Transcript paths.** `AgentAdapter::validate_transcript_path(path)` returns the
  resolved path only if it is absolute, ends in `.jsonl`, and, with symlinks resolved,
  lies under `transcript_root()`. For a file that does not exist yet, the deepest existing
  directory is resolved and the rest must be plain names. The roots are: Claude Code
  `<config>/projects`, Cursor `~/.cursor/projects`, Codex `$CODEX_HOME/sessions`; other
  agents have none, so every path is rejected. The receiver applies this check to
  `transcript_path` and to the subagent transcript path of every event. The daemon should
  also apply it to paths it stored earlier (database, persisted tracker) before reading
  them.

## Adapter checklist for another CLI

Fill in for each agent before calling it supported (`AgentAdapter` in
`crates/muster-agents/src/adapter.rs`):

1. **Install check and launch command**: executable name, how to pass an initial
   prompt safely (option terminator), extra args.
2. **Session id**: can Muster choose it at launch? If not, which event or file reveals
   it, and how to map a card to it after a reboot.
3. **Resume command**: by id; does resume keep the id or fork a new one?
4. **Hook mechanism without editing user config**: per-launch flag, env var, or
   config-dir override. If only a config edit works: consent prompt, exact-match
   reversible install/uninstall, tested only against a temp dir.
5. **Event map**: real event names and payload fields (captured from a run, saved as
   fixtures) for turn start, turn end, tool start/end, permission/question waits, session
   start/end, session id changes, subagents. Note which state changes produce no event
   (interrupts, crashes) and how to detect them instead (transcript, terminal output).
6. **Permission answers**: can a hook return allow/deny? Is the CLI's own prompt shown
   meanwhile? What happens to the hook when the terminal answers first (killed, or left
   running)? Exact decision output. Terminal key fallback and its caveats.
7. **Trust/onboarding screens** that appear before hooks run.
8. **Transcript**: location, format, how to page from the end, which lines are chat,
   how tool calls and reasoning appear, what to skip.
9. **Context usage**: tokens used and window size, and where each comes from.
10. **Usage limits**: least invasive source, with the same order as above.
11. **Fixtures and tests**: sanitised payloads and transcript excerpts under
    `tests/fixtures/<agent>/`, with normalisation, status and transcript tests.
12. **Hook authenticity**: pass `LaunchSpec::hook_secret` to the session as
    `MUSTER_HOOK_SECRET` and implement `transcript_root()`.
13. **Facts recorded here**, each marked `[run]` or `[docs]`, with the CLI version.
