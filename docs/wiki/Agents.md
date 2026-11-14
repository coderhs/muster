# Agents

Muster starts each agent's own command-line tool. It does not replace or wrap the
agent's interface. How much Muster knows about a card depends on what the tool
exposes.

## Summary

| | Claude Code | Codex | Cursor | opencode | Gemini | Antigravity | Shell |
|---|---|---|---|---|---|---|---|
| Start with a prompt | yes | yes | yes | yes | yes | yes | command |
| Resume | yes | yes | yes | yes | yes | no | – |
| Status from | hooks | its session log | hooks | its local API | terminal activity | terminal activity | terminal activity |
| "Needs you" | yes | no | no | no | by quiet time | by quiet time | – |
| Answer approvals from the card | yes | no | no | no | no | no | – |
| Chat view | yes | yes | text only | yes | no | no | – |
| Context meter | yes | yes | tokens | tokens | no | no | – |
| Usage limits | yes | from its log | no | no | no | no | – |
| Model on the card | yes | yes | yes | yes | no | no | – |
| Signed-in check | yes | yes | yes | yes | yes | no | – |

How well each row has been confirmed against the real tool is recorded in
[`docs/AGENTS.md`](../AGENTS.md). Claude Code and Cursor were run for real; the others
are partly built from their documentation.

## Claude Code, the deepest integration

- **No config edits.** Muster starts Claude with a generated settings file for that
  launch only. Your `~/.claude` is not touched.
- **Session id.** Muster chooses the session id, so it can resume the conversation
  later with `--resume`.
- **Hooks.** Events such as prompt submitted, tool approval requested and turn
  finished set the card's status.
- **Approvals.** A permission request appears on the card and in the terminal at the
  same time. Whichever is answered first wins.
- **Questions and plans.** Can be answered from the card.
- **Usage limits.** Read from the status-line data Claude provides. If you already
  have a status line, Muster passes the data on to it. Each session reports the limits
  from its own last request, so Muster merges every card's reports per window instead of
  taking the latest one: a session left idle for hours can no longer roll the numbers
  back or hide the 5-hour window.
- **Instructions.** Muster adds a short note and a `muster` skill so the agent knows
  how to read linked cards and message other agents.

## Other agents

- **Cursor**: hooks through a generated plug-in folder. Approvals stay in the terminal.
- **Codex**: status is read from the session log Codex writes. Hooks are off.
- **opencode**: Muster asks opencode's local API at status changes and when the chat
  view opens, not continuously.
- **Gemini, Antigravity**: started and (for Gemini) resumed; status comes from
  terminal activity.
- **Shell**: a plain terminal card, optionally running a command.

## Sign-in check

The tracker's agents panel says whether each tool is signed in. Muster looks only at
local state and never reads a token: Claude Code's account entry in `~/.claude.json`,
`codex login status`, the signed-in user in `~/.cursor/cli-config.json`, Gemini CLI's
settings and whether its login file exists, and the provider names in opencode's
database. Antigravity keeps its login somewhere Muster cannot see, so it shows
"Sign-in unknown". Cursor, Gemini and opencode keep plan usage on their servers only, so
no limits are shown for them.

## Adding an agent

An adapter lives in `crates/muster-agents`. It answers a small set of questions: how
to launch, how to resume, how to install hooks for one launch, how to turn the tool's
events into Muster events, and where the transcript is. Add the adapter, register it
in the daemon's agent list, and record what was verified in `docs/AGENTS.md`.
